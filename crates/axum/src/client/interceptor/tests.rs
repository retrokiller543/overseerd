use std::sync::Mutex;

use http::header::HeaderValue;
use http::{StatusCode, request, response};
use overseerd_client::ClientError;

use super::ClientInterceptor;

#[derive(Default)]
struct RecordingInterceptor {
    errors: Mutex<Vec<String>>,
}

impl ClientInterceptor for RecordingInterceptor {
    fn on_request(&self, request: &mut request::Parts) {
        request
            .headers
            .insert("authorization", HeaderValue::from_static("Bearer test"));
    }

    fn on_response(&self, response: &mut response::Parts) {
        response
            .headers
            .insert("x-intercepted", HeaderValue::from_static("yes"));
    }

    fn on_error<E>(&self, error: &ClientError<StatusCode, E>) {
        self.errors.lock().unwrap().push(error.to_string());
    }
}

#[test]
fn interceptor_mutates_standard_http_parts_and_observes_errors() {
    let interceptor = RecordingInterceptor::default();
    let (mut request, _) = http::Request::new(()).into_parts();
    interceptor.on_request(&mut request);
    assert_eq!(request.headers["authorization"], "Bearer test");

    let (mut response, _) = http::Response::new(()).into_parts();
    interceptor.on_response(&mut response);
    assert_eq!(response.headers["x-intercepted"], "yes");

    let error: ClientError<StatusCode> = ClientError::Decode("bad response".to_owned());
    interceptor.on_error(&error);
    assert_eq!(interceptor.errors.lock().unwrap().len(), 1);
}
