use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use vpr_integration::{
    CancellationProbe, ProviderDescriptor, ProviderError, ProviderErrorKind,
    RealtimeAvatarCapabilities, RealtimeAvatarCapability, RealtimeAvatarPort,
    RealtimeAvatarSession, RealtimeAvatarTransport, WebRtcIceCandidate, WebRtcIceServer,
    WebRtcSessionDescription,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct LocalOpenSourceAvatarConfig {
    endpoint: String,
    api_token: String,
    timeout: Duration,
}

impl LocalOpenSourceAvatarConfig {
    #[must_use]
    pub fn new(endpoint: impl Into<String>, api_token: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            api_token: api_token.into(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

pub struct LocalOpenSourceAvatar {
    client: Client,
    base_url: reqwest::Url,
    api_token: String,
}

impl LocalOpenSourceAvatar {
    /// Builds the self-hosted realtime avatar adapter.
    ///
    /// The worker contract is intentionally provider-neutral: the remote GPU worker may run
    /// MuseTalk, LivePortrait, or another local renderer behind the same WebRTC API.
    ///
    /// # Errors
    /// Returns a typed provider error for invalid endpoints, missing authentication, or HTTP setup.
    pub fn new(config: LocalOpenSourceAvatarConfig) -> Result<Self, ProviderError> {
        let base_url = validate_endpoint(&config.endpoint)?;
        if config.api_token.trim().is_empty() {
            return Err(invalid_response());
        }
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| map_transport_error(&error))?;
        Ok(Self {
            client,
            base_url,
            api_token: config.api_token,
        })
    }

    fn sessions_url(&self) -> Result<reqwest::Url, ProviderError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|()| invalid_response())?
            .extend(["v1", "avatar", "sessions"]);
        Ok(url)
    }

    fn session_url(&self, session_id: &str) -> Result<reqwest::Url, ProviderError> {
        let mut url = self.sessions_url()?;
        url.path_segments_mut()
            .map_err(|()| invalid_response())?
            .push(session_id);
        Ok(url)
    }

    fn subresource_url(
        &self,
        session_id: &str,
        resource: &str,
    ) -> Result<reqwest::Url, ProviderError> {
        let mut url = self.session_url(session_id)?;
        url.path_segments_mut()
            .map_err(|()| invalid_response())?
            .push(resource);
        Ok(url)
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        request
            .header(AUTHORIZATION, format!("Bearer {}", self.api_token))
            .header(CONTENT_TYPE, "application/json")
    }

    fn validate_session(session: &RealtimeAvatarSession) -> Result<&str, ProviderError> {
        let id = session.provider_session_id.trim();
        if id.is_empty() || session.provider_resource_id != session.provider_session_id {
            return Err(invalid_response());
        }
        if !matches!(session.transport, RealtimeAvatarTransport::WebRtc { .. }) {
            return Err(invalid_response());
        }
        Ok(id)
    }

    fn ensure_active(cancellation: &dyn CancellationProbe) -> Result<(), ProviderError> {
        if cancellation.is_cancelled() {
            Err(cancelled())
        } else {
            Ok(())
        }
    }
}

impl RealtimeAvatarPort for LocalOpenSourceAvatar {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "local-open-source".to_owned(),
            model: "realtime-worker-v1".to_owned(),
            representation: None,
        }
    }

    fn capabilities(&self) -> RealtimeAvatarCapabilities {
        RealtimeAvatarCapabilities::new([
            RealtimeAvatarCapability::TextInput,
            RealtimeAvatarCapability::Interrupt,
        ])
    }

    fn create_session(
        &self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        Self::ensure_active(cancellation)?;
        let response = self
            .authorized(self.client.post(self.sessions_url()?))
            .json(&CreateSessionRequest {})
            .send()
            .map_err(|error| map_transport_error(&error))?;
        let response = expect_success(response)?;
        let body: CreateSessionResponse = response.json().map_err(|_| invalid_response())?;
        body.try_into()
    }

    fn submit_answer(
        &self,
        session: &RealtimeAvatarSession,
        answer: &WebRtcSessionDescription,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Self::ensure_active(cancellation)?;
        let id = Self::validate_session(session)?;
        if answer.kind != "answer" || answer.sdp.trim().is_empty() {
            return Err(invalid_response());
        }
        let response = self
            .authorized(self.client.post(self.subresource_url(id, "answer")?))
            .json(&AnswerRequest {
                kind: &answer.kind,
                sdp: &answer.sdp,
            })
            .send()
            .map_err(|error| map_transport_error(&error))?;
        expect_success(response).map(|_| ())
    }

    fn submit_ice_candidate(
        &self,
        session: &RealtimeAvatarSession,
        candidate: &WebRtcIceCandidate,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Self::ensure_active(cancellation)?;
        let id = Self::validate_session(session)?;
        let response = self
            .authorized(self.client.post(self.subresource_url(id, "ice")?))
            .json(&IceRequest {
                candidate: candidate.candidate.as_deref(),
                sdp_mid: candidate.sdp_mid.as_deref(),
                sdp_mline_index: candidate.sdp_mline_index,
            })
            .send()
            .map_err(|error| map_transport_error(&error))?;
        expect_success(response).map(|_| ())
    }

    fn speak_text(
        &self,
        session: &RealtimeAvatarSession,
        text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Self::ensure_active(cancellation)?;
        let id = Self::validate_session(session)?;
        let text = text.trim();
        if text.is_empty() {
            return Err(invalid_response());
        }
        let response = self
            .authorized(self.client.post(self.subresource_url(id, "speak")?))
            .json(&SpeakRequest { text })
            .send()
            .map_err(|error| map_transport_error(&error))?;
        expect_success(response).map(|_| ())
    }

    fn speak_audio_url(
        &self,
        _session: &RealtimeAvatarSession,
        _audio_url: &str,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Err(unavailable())
    }

    fn interrupt(
        &self,
        session: &RealtimeAvatarSession,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Self::ensure_active(cancellation)?;
        let id = Self::validate_session(session)?;
        let response = self
            .authorized(self.client.post(self.subresource_url(id, "interrupt")?))
            .json(&InterruptRequest {})
            .send()
            .map_err(|error| map_transport_error(&error))?;
        expect_success(response).map(|_| ())
    }

    fn close_session(&self, session: &RealtimeAvatarSession) -> Result<(), ProviderError> {
        let id = Self::validate_session(session)?;
        let response = self
            .authorized(self.client.delete(self.session_url(id)?))
            .send()
            .map_err(|error| map_transport_error(&error))?;
        expect_success(response).map(|_| ())
    }
}

#[derive(Serialize)]
struct CreateSessionRequest {}

#[derive(Deserialize)]
struct CreateSessionResponse {
    id: String,
    offer: SessionDescription,
    #[serde(default)]
    ice_servers: Vec<IceServer>,
}

#[derive(Deserialize)]
struct SessionDescription {
    #[serde(rename = "type")]
    kind: String,
    sdp: String,
}

#[derive(Deserialize)]
struct IceServer {
    #[serde(default)]
    urls: OneOrMany,
    username: Option<String>,
    credential: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
    #[default]
    Missing,
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
            Self::Missing => Vec::new(),
        }
    }
}

impl TryFrom<CreateSessionResponse> for RealtimeAvatarSession {
    type Error = ProviderError;

    fn try_from(value: CreateSessionResponse) -> Result<Self, Self::Error> {
        let id = value.id.trim();
        if id.is_empty() || value.offer.kind != "offer" || value.offer.sdp.trim().is_empty() {
            return Err(invalid_response());
        }
        Ok(Self {
            provider_resource_id: id.to_owned(),
            provider_session_id: id.to_owned(),
            transport: RealtimeAvatarTransport::WebRtc {
                offer: WebRtcSessionDescription {
                    kind: value.offer.kind,
                    sdp: value.offer.sdp,
                },
                ice_servers: value
                    .ice_servers
                    .into_iter()
                    .map(|server| WebRtcIceServer {
                        urls: server.urls.into_vec(),
                        username: server.username,
                        credential: server.credential,
                    })
                    .collect(),
            },
        })
    }
}

#[derive(Serialize)]
struct AnswerRequest<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    sdp: &'a str,
}

#[derive(Serialize)]
struct IceRequest<'a> {
    candidate: Option<&'a str>,
    #[serde(rename = "sdpMid")]
    sdp_mid: Option<&'a str>,
    #[serde(rename = "sdpMLineIndex")]
    sdp_mline_index: Option<u16>,
}

#[derive(Serialize)]
struct SpeakRequest<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct InterruptRequest {}

fn validate_endpoint(endpoint: &str) -> Result<reqwest::Url, ProviderError> {
    let parsed = reqwest::Url::parse(endpoint).map_err(|_| invalid_response())?;
    let loopback = parsed.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    let secure = parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback);
    if secure
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
    {
        Ok(parsed)
    } else {
        Err(policy_denied())
    }
}

fn expect_success(response: Response) -> Result<Response, ProviderError> {
    if response.status().is_success() {
        return Ok(response);
    }
    Err(match response.status().as_u16() {
        401 | 403 => policy_denied(),
        408 => ProviderError {
            kind: ProviderErrorKind::Timeout,
            retryable: true,
        },
        429 => ProviderError {
            kind: ProviderErrorKind::RateLimited,
            retryable: true,
        },
        500..=599 => ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: true,
        },
        _ => invalid_response(),
    })
}

fn map_transport_error(error: &reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError {
            kind: ProviderErrorKind::Timeout,
            retryable: true,
        }
    } else {
        ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: true,
        }
    }
}

const fn invalid_response() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::InvalidResponse,
        retryable: false,
    }
}

const fn unavailable() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Unavailable,
        retryable: false,
    }
}

const fn policy_denied() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::PolicyDenied,
        retryable: false,
    }
}

const fn cancelled() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Cancelled,
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::thread;

    struct Probe(AtomicBool);

    impl CancellationProbe for Probe {
        fn is_cancelled(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        let mut total = None;
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if total.is_none()
                && let Some(header_end) =
                    request.windows(4).position(|window| window == b"\r\n\r\n")
            {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                total = Some(header_end + 4 + content_length);
            }
            if total.is_some_and(|expected| request.len() >= expected) {
                break;
            }
        }
        String::from_utf8_lossy(&request).into_owned()
    }

    fn serve(responses: Vec<(&'static str, String)>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = tx.send(read_request(&mut stream));
                let headers = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(headers.as_bytes()).unwrap();
                stream.write_all(body.as_bytes()).unwrap();
            }
        });
        (format!("http://{address}"), rx)
    }

    fn provider(endpoint: String) -> LocalOpenSourceAvatar {
        LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(endpoint, "worker-secret"))
            .unwrap()
    }

    #[test]
    fn full_worker_control_plane_matches_contract() {
        let create = r#"{"id":"local-1","offer":{"type":"offer","sdp":"offer-sdp"},"ice_servers":[{"urls":"stun:example.test"}]}"#;
        let (endpoint, captured) = serve(vec![
            ("201 Created", create.to_owned()),
            ("200 OK", "{}".to_owned()),
            ("200 OK", "{}".to_owned()),
            ("200 OK", "{}".to_owned()),
            ("200 OK", "{}".to_owned()),
            ("200 OK", "{}".to_owned()),
        ]);
        let avatar = provider(endpoint);
        let probe = Probe(AtomicBool::new(false));
        let session = avatar.create_session(&probe).unwrap();
        avatar
            .submit_answer(
                &session,
                &WebRtcSessionDescription {
                    kind: "answer".to_owned(),
                    sdp: "answer-sdp".to_owned(),
                },
                &probe,
            )
            .unwrap();
        avatar
            .submit_ice_candidate(
                &session,
                &WebRtcIceCandidate {
                    candidate: Some("candidate-x".to_owned()),
                    sdp_mid: Some("0".to_owned()),
                    sdp_mline_index: Some(0),
                },
                &probe,
            )
            .unwrap();
        avatar.speak_text(&session, "Привет", &probe).unwrap();
        avatar.interrupt(&session, &probe).unwrap();
        avatar.close_session(&session).unwrap();

        let requests: Vec<String> = (0..6).map(|_| captured.recv().unwrap()).collect();
        assert!(requests[0].starts_with("POST /v1/avatar/sessions "));
        assert!(requests[1].starts_with("POST /v1/avatar/sessions/local-1/answer "));
        assert!(requests[2].starts_with("POST /v1/avatar/sessions/local-1/ice "));
        assert!(requests[3].starts_with("POST /v1/avatar/sessions/local-1/speak "));
        assert!(requests[4].starts_with("POST /v1/avatar/sessions/local-1/interrupt "));
        assert!(requests[5].starts_with("DELETE /v1/avatar/sessions/local-1 "));
        assert!(
            requests
                .iter()
                .all(|request| request.contains("Authorization: Bearer worker-secret"))
        );
        assert!(requests[3].contains("Привет"));
    }

    #[test]
    fn remote_plain_http_is_rejected() {
        let error = LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(
            "http://example.com",
            "token",
        ))
        .err()
        .expect("remote plaintext must fail");
        assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
    }

    #[test]
    fn cancellation_prevents_network_work() {
        let avatar = provider("http://127.0.0.1:1".to_owned());
        let error = avatar
            .create_session(&Probe(AtomicBool::new(true)))
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    }
}
