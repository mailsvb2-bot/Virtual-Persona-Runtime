use std::net::TcpStream;
use std::time::Duration;

use serde::Deserialize;
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};
use vpr_integration::{
    CancellationProbe, ProviderError, ProviderErrorKind, SttAudioStream, SttStreamEvent,
    SttStreamRequest, Transcript, UsageEvidence, UsageUnit,
};

use crate::{
    DeepgramSttConfig, cancelled, invalid_response, map_status, normalize_deepgram_language,
};

const IO_POLL_TIMEOUT: Duration = Duration::from_millis(100);

pub(crate) fn open(
    config: &DeepgramSttConfig,
    request: &SttStreamRequest,
    cancellation: &dyn CancellationProbe,
) -> Result<Box<dyn SttAudioStream>, ProviderError> {
    if cancellation.is_cancelled() {
        return Err(cancelled());
    }
    if !request.is_well_formed() {
        return Err(invalid_response());
    }

    let url = streaming_url(config, request)?;
    let mut websocket_request = url
        .as_str()
        .into_client_request()
        .map_err(|_| invalid_response())?;
    websocket_request.headers_mut().insert(
        "Authorization",
        format!("Token {}", config.api_key)
            .parse()
            .map_err(|_| invalid_response())?,
    );

    let (mut socket, _response) =
        tungstenite::connect(websocket_request).map_err(map_websocket_error)?;
    set_io_timeouts(socket.get_mut(), config.timeout)?;

    Ok(Box::new(DeepgramLiveStream {
        socket,
        request: request.clone(),
        input_bytes: 0,
        input_finished: false,
        completed: false,
    }))
}

struct DeepgramLiveStream {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    request: SttStreamRequest,
    input_bytes: u64,
    input_finished: bool,
    completed: bool,
}

impl SttAudioStream for DeepgramLiveStream {
    fn push_audio(
        &mut self,
        pcm: &[u8],
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        self.ensure_active(cancellation)?;
        if self.input_finished || !self.request.is_well_formed_chunk(pcm) {
            return Err(invalid_response());
        }
        self.socket
            .send(Message::Binary(pcm.to_vec().into()))
            .map_err(map_websocket_error)?;
        self.input_bytes = self
            .input_bytes
            .checked_add(u64::try_from(pcm.len()).map_err(|_| invalid_response())?)
            .ok_or_else(invalid_response)?;
        Ok(())
    }

    fn finish_input(&mut self, cancellation: &dyn CancellationProbe) -> Result<(), ProviderError> {
        self.ensure_active(cancellation)?;
        if self.input_finished {
            return Ok(());
        }
        self.socket
            .send(Message::Text(r#"{"type":"CloseStream"}"#.into()))
            .map_err(map_websocket_error)?;
        self.input_finished = true;
        Ok(())
    }

    fn next_event(
        &mut self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Option<SttStreamEvent>, ProviderError> {
        loop {
            self.ensure_active(cancellation)?;
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    if let Some(event) = parse_event(&text, self.request.locale_hint.as_deref())? {
                        return Ok(Some(event));
                    }
                }
                Ok(Message::Ping(payload)) => {
                    self.socket
                        .send(Message::Pong(payload))
                        .map_err(map_websocket_error)?;
                }
                Ok(Message::Pong(_) | Message::Binary(_) | Message::Frame(_)) => {}
                Ok(Message::Close(_)) => {
                    self.completed = true;
                    return Ok(None);
                }
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    if cancellation.is_cancelled() {
                        return Err(cancelled());
                    }
                }
                Err(error) => return Err(map_websocket_error(error)),
            }
        }
    }

    fn usage(&self) -> UsageEvidence {
        UsageEvidence {
            input_units: input_duration_millis(&self.request, self.input_bytes),
            input_unit: Some(UsageUnit::AudioMillisecond),
            ..UsageEvidence::default()
        }
    }
}

impl DeepgramLiveStream {
    fn ensure_active(&self, cancellation: &dyn CancellationProbe) -> Result<(), ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if self.completed {
            return Err(ProviderError {
                kind: ProviderErrorKind::Unavailable,
                retryable: false,
            });
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct LiveMessage {
    #[serde(rename = "type")]
    message_type: Option<String>,
    #[serde(default)]
    is_final: bool,
    channel: Option<LiveChannel>,
}

#[derive(Deserialize)]
struct LiveChannel {
    alternatives: Vec<LiveAlternative>,
}

#[derive(Deserialize)]
struct LiveAlternative {
    transcript: String,
}

fn parse_event(
    text: &str,
    locale_hint: Option<&str>,
) -> Result<Option<SttStreamEvent>, ProviderError> {
    let message: LiveMessage = serde_json::from_str(text).map_err(|_| invalid_response())?;
    match message.message_type.as_deref() {
        Some("Results") | None => {}

        Some("Error") => return Err(invalid_response()),
        Some(_) => return Ok(None),
    }
    let Some(channel) = message.channel else {
        return Ok(None);
    };
    let Some(alternative) = channel.alternatives.first() else {
        return Ok(None);
    };
    if alternative.transcript.trim().is_empty() {
        return Ok(None);
    }
    let transcript = Transcript {
        text: alternative.transcript.clone(),
        locale: locale_hint
            .map_or("und", normalize_deepgram_language)
            .to_owned(),
    };
    Ok(Some(if message.is_final {
        SttStreamEvent::Final(transcript)
    } else {
        SttStreamEvent::Interim(transcript)
    }))
}

fn streaming_url(
    config: &DeepgramSttConfig,
    request: &SttStreamRequest,
) -> Result<reqwest::Url, ProviderError> {
    let mut url = reqwest::Url::parse(&config.endpoint).map_err(|_| invalid_response())?;
    match url.scheme() {
        "https" => {
            url.set_scheme("wss").map_err(|()| invalid_response())?;
        }
        "http" if is_loopback(&url) => {
            url.set_scheme("ws").map_err(|()| invalid_response())?;
        }
        "wss" => {}
        "ws" if is_loopback(&url) => {}
        _ => {
            return Err(ProviderError {
                kind: ProviderErrorKind::PolicyDenied,
                retryable: false,
            });
        }
    }
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("model", &config.model);
        query.append_pair("encoding", "linear16");
        query.append_pair("sample_rate", &request.sample_rate_hz.to_string());
        query.append_pair("channels", &request.channels.to_string());
        query.append_pair("interim_results", "true");
        query.append_pair(
            "smart_format",
            if config.smart_format { "true" } else { "false" },
        );
        if let Some(locale) = request
            .locale_hint
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            query.append_pair("language", normalize_deepgram_language(locale));
        }
    }
    Ok(url)
}

fn is_loopback(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}

fn set_io_timeouts(
    stream: &mut MaybeTlsStream<TcpStream>,
    overall_timeout: Duration,
) -> Result<(), ProviderError> {
    let poll = IO_POLL_TIMEOUT.min(overall_timeout);
    match stream {
        MaybeTlsStream::Plain(stream) => {
            stream
                .set_read_timeout(Some(poll))
                .map_err(|_| unavailable())?;
            stream
                .set_write_timeout(Some(overall_timeout))
                .map_err(|_| unavailable())?;
        }
        MaybeTlsStream::Rustls(stream) => {
            let tcp = stream.get_mut();
            tcp.set_read_timeout(Some(poll))
                .map_err(|_| unavailable())?;
            tcp.set_write_timeout(Some(overall_timeout))
                .map_err(|_| unavailable())?;
        }
        _ => return Err(unavailable()),
    }
    Ok(())
}

fn input_duration_millis(request: &SttStreamRequest, bytes: u64) -> Option<u64> {
    let frame_bytes = u64::from(request.channels)
        .checked_mul(u64::try_from(request.sample_format.bytes_per_sample()).ok()?)?;
    let bytes_per_second = u64::from(request.sample_rate_hz).checked_mul(frame_bytes)?;
    bytes.checked_mul(1_000)?.checked_div(bytes_per_second)
}

fn map_websocket_error(error: tungstenite::Error) -> ProviderError {
    match error {
        tungstenite::Error::Http(response) => map_status(response.status().as_u16()),
        tungstenite::Error::Io(error) if error.kind() == std::io::ErrorKind::TimedOut => {
            ProviderError {
                kind: ProviderErrorKind::Timeout,
                retryable: true,
            }
        }
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: true,
        },
        _ => unavailable(),
    }
}

fn unavailable() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Unavailable,
        retryable: true,
    }
}
