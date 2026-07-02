//! Tests for the NDJSON line decoder (see the parent [`super`] module).

use bytes::Bytes;
use futures::StreamExt;

use super::{MAX_NDJSON_LINE_BYTES, ndjson_decode};

#[tokio::test]
async fn ndjson_decode_stops_on_oversized_line_without_newline() {
    let body = futures::stream::iter([Ok::<_, std::convert::Infallible>(Bytes::from(vec![
        b'x';
        MAX_NDJSON_LINE_BYTES + 1
    ]))]);
    let mut decoded = Box::pin(ndjson_decode::<_, _, String>(body));

    assert!(decoded.next().await.is_none());
}

#[tokio::test]
async fn ndjson_decode_stops_on_oversized_line_with_newline() {
    let mut line = vec![b'x'; MAX_NDJSON_LINE_BYTES + 1];
    line.push(b'\n');
    let body = futures::stream::iter([Ok::<_, std::convert::Infallible>(Bytes::from(line))]);
    let mut decoded = Box::pin(ndjson_decode::<_, _, String>(body));

    assert!(decoded.next().await.is_none());
}
