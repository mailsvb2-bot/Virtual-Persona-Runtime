use std::io::Read;

use serde::Deserialize;
use tiny_http::Request;

use super::{HttpResponse, MAX_BODY_BYTES, error_response, is_json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyJsonBody {}

pub(crate) fn parse_json<T: for<'de> Deserialize<'de>>(
    request: &mut Request,
) -> Result<T, HttpResponse> {
    if !is_json(request) {
        return Err(error_response(415, "JSON_REQUIRED"));
    }
    let body = read_body(request, MAX_BODY_BYTES)?;
    serde_json::from_slice(&body).map_err(|_| error_response(400, "INVALID_INPUT"))
}

pub(crate) fn parse_empty_json(request: &mut Request) -> Result<(), HttpResponse> {
    parse_json::<EmptyJsonBody>(request).map(|_| ())
}

pub(crate) fn read_body(request: &mut Request, limit: u64) -> Result<Vec<u8>, HttpResponse> {
    let mut body = Vec::new();
    request
        .as_reader()
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|_| error_response(400, "INVALID_INPUT"))?;
    if body.len() as u64 > limit {
        return Err(error_response(413, "BODY_TOO_LARGE"));
    }
    Ok(body)
}
