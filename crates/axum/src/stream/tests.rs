//! Tests for the NDJSON line decoder (see the parent [`super`] module).

use bytes::Bytes;
use futures::StreamExt;

use super::{
    MAX_NDJSON_LINE_BYTES, StreamRequestLimits, limit_request_body, limited_ndjson_decode,
    ndjson_decode,
};

#[tokio::test]
async fn ndjson_decode_stops_on_oversized_line_without_newline() {
    let body = futures::stream::iter([Ok::<_, std::convert::Infallible>(Bytes::from(vec![
        b'x';
        MAX_NDJSON_LINE_BYTES
            + 1
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

#[tokio::test]
async fn streamed_request_stops_before_a_chunk_exceeds_the_total_byte_limit() {
    let body = futures::stream::iter([
        Ok::<_, std::convert::Infallible>(Bytes::from_static(b"1\n")),
        Ok(Bytes::from_static(b"22\n")),
    ]);
    let limits = StreamRequestLimits {
        max_bytes: 3,
        max_items: 0,
        timeout: None,
    };
    let chunks: Vec<_> = limit_request_body(body, limits)
        .map(Result::unwrap)
        .collect()
        .await;

    assert_eq!(chunks, vec![Bytes::from_static(b"1\n")]);
}

#[tokio::test(start_paused = true)]
async fn streamed_request_deadline_terminates_an_idle_input() {
    let body = futures::stream::pending::<Result<Bytes, std::convert::Infallible>>();
    let limits = StreamRequestLimits {
        max_bytes: 0,
        max_items: 0,
        timeout: Some(std::time::Duration::from_secs(5)),
    };
    let task = tokio::spawn(async move {
        let mut body = Box::pin(limit_request_body(body, limits));

        body.next().await
    });

    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(5)).await;

    assert!(task.await.expect("limit task did not panic").is_none());
}

#[tokio::test(start_paused = true)]
async fn streamed_request_deadline_preempts_an_immediately_ready_input() {
    let body = futures::stream::repeat(Ok::<_, std::convert::Infallible>(Bytes::from_static(
        b"1\n",
    )));
    let limits = StreamRequestLimits {
        max_bytes: 0,
        max_items: 0,
        timeout: Some(std::time::Duration::from_secs(5)),
    };
    let mut body = Box::pin(limit_request_body(body, limits));

    assert!(body.next().await.is_some(), "body is ready before deadline");

    tokio::time::advance(std::time::Duration::from_secs(5)).await;

    assert!(
        body.next().await.is_none(),
        "ready chunks cannot win once the total deadline has elapsed"
    );
}

#[tokio::test(start_paused = true)]
async fn streamed_request_deadline_preempts_items_buffered_before_the_deadline() {
    let body = futures::stream::iter([Ok::<_, std::convert::Infallible>(Bytes::from_static(
        b"1\n2\n",
    ))]);
    let limits = StreamRequestLimits {
        max_bytes: 0,
        max_items: 0,
        timeout: Some(std::time::Duration::from_secs(5)),
    };
    let mut decoded = limited_ndjson_decode::<_, _, u64>(body, limits);

    assert_eq!(decoded.next().await, Some(1));

    tokio::time::advance(std::time::Duration::from_secs(5)).await;

    assert_eq!(
        decoded.next().await,
        None,
        "a decoded record buffered in an earlier chunk cannot outlive the total deadline"
    );
}

#[tokio::test]
async fn item_limit_completes_without_leaving_a_body_consumer_task() {
    let body = futures::stream::iter([Ok::<_, std::convert::Infallible>(Bytes::from_static(
        b"1\n",
    ))])
    .chain(futures::stream::pending());
    let limits = StreamRequestLimits {
        max_bytes: 0,
        max_items: 1,
        timeout: None,
    };
    let task = tokio::spawn(async move {
        limited_ndjson_decode::<_, _, u64>(body, limits)
            .collect::<Vec<_>>()
            .await
    });
    let items = task.await.expect("bounded handler task completed");

    assert_eq!(items, vec![1]);
}
