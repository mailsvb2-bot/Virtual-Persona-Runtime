use std::io::Read;
use std::net::TcpStream;
use std::time::Duration;

pub fn read_complete_http_request(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut request = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "connection closed before HTTP headers completed");
        request.extend_from_slice(&chunk[..read]);
        if let Some(position) = request.windows(4).position(|part| part == b"\r\n\r\n") {
            break position + 4;
        }
    };

    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    if let Some(content_length) = content_length {
        let target = header_end + content_length;
        while request.len() < target {
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0, "connection closed before HTTP body completed");
            request.extend_from_slice(&chunk[..read]);
        }
        return;
    }

    let chunked = headers.lines().any(|line| {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|coding| coding.trim().eq_ignore_ascii_case("chunked"))
    });
    if chunked {
        while !request[header_end..]
            .windows(5)
            .any(|part| part == b"0\r\n\r\n")
        {
            let read = stream.read(&mut chunk).unwrap();
            assert!(
                read > 0,
                "connection closed before chunked HTTP body completed"
            );
            request.extend_from_slice(&chunk[..read]);
        }
    }
}
