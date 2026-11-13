//! Local browser assets. Close each static HTTP/1.1 response so an idle
//! keep-alive socket cannot hold a slot in the browser's bounded module pool.
use tiny_http::Header;

pub(super) fn static_response(body: &str, content_type: &str) -> super::HttpResponse {
    let mut reply = super::response(200, body.as_bytes().to_vec(), content_type);
    // Fixed literals are a valid header; no external input enters this path.
    reply.add_header(Header::from_bytes("Connection", "close").expect("valid static response header"));
    reply
}
