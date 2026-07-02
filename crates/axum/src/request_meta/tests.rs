use axum::http::{HeaderMap, HeaderValue, Method, Uri, header};

use super::RequestMeta;

#[test]
fn parses_a_single_cookie() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_static("session_id=abc123"),
    );

    let meta = RequestMeta::from_parts(Method::GET, Uri::from_static("/"), headers);

    assert_eq!(meta.cookies.get("session_id"), Some(&"abc123".to_string()));
}

#[test]
fn parses_multiple_cookies_in_one_header() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_static("a=1; b=2; session_id=abc123"),
    );

    let meta = RequestMeta::from_parts(Method::GET, Uri::from_static("/"), headers);

    assert_eq!(meta.cookies.get("a"), Some(&"1".to_string()));
    assert_eq!(meta.cookies.get("b"), Some(&"2".to_string()));
    assert_eq!(meta.cookies.get("session_id"), Some(&"abc123".to_string()));
}

#[test]
fn no_cookie_header_yields_empty_map() {
    let meta = RequestMeta::from_parts(Method::GET, Uri::from_static("/"), HeaderMap::new());

    assert!(meta.cookies.is_empty());
}
