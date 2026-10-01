use std::time::Duration;

use reqwest::blocking::Client;

/// Builds the canonical blocking HTTP client used by provider adapters.
///
/// Provider endpoints are validated by each adapter before requests are built. Redirects are
/// deliberately disabled here so an accepted endpoint cannot redirect credentials or request
/// bodies to a second destination that never passed that endpoint policy.
///
/// # Errors
/// Returns reqwest's client-construction error.
pub fn build_provider_http_client(timeout: Duration) -> Result<Client, reqwest::Error> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use reqwest::header::AUTHORIZATION;

    use super::build_provider_http_client;

    fn serve_once(status: &'static str, extra_headers: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            let response = format!(
                "HTTP/1.1 {status}\r\n{extra_headers}Content-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{address}/provider")
    }

    #[test]
    fn direct_loopback_request_remains_supported() {
        let endpoint = serve_once("204 No Content", String::new());
        let response = build_provider_http_client(Duration::from_secs(2))
            .unwrap()
            .get(endpoint)
            .send()
            .unwrap();
        assert_eq!(response.status().as_u16(), 204);
    }

    #[test]
    fn redirects_are_returned_without_following_or_leaking_authorization() {
        for status in ["307 Temporary Redirect", "308 Permanent Redirect"] {
            let target = TcpListener::bind("127.0.0.1:0").unwrap();
            target.set_nonblocking(true).unwrap();
            let target_url = format!("http://{}/capture", target.local_addr().unwrap());
            let endpoint = serve_once(status, format!("Location: {target_url}\r\n"));

            let response = build_provider_http_client(Duration::from_secs(2))
                .unwrap()
                .post(endpoint)
                .header(AUTHORIZATION, "Bearer must-not-cross-origin")
                .body("sensitive-body")
                .send()
                .unwrap();

            assert_eq!(
                response.status().as_u16(),
                if status.starts_with("307") { 307 } else { 308 }
            );
            thread::sleep(Duration::from_millis(25));
            match target.accept() {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Ok(_) => panic!("redirect target received a request despite no-redirect policy"),
                Err(error) => panic!("unexpected redirect-target accept error: {error}"),
            }
        }
    }
}
