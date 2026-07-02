//! Tests for the [`Broker`](super::Broker) registry and fan-out.

use super::*;

fn body(text: &str) -> StompBody {
    StompBody::json(text.as_bytes().to_vec())
}

#[tokio::test]
async fn publish_reaches_only_matching_subscribers() {
    let broker = Broker::new();
    let conn_a = broker.register();
    let conn_b = broker.register();
    let (tx_a, mut rx_a) = mpsc::channel(4);
    let (tx_b, mut rx_b) = mpsc::channel(4);

    broker.subscribe(conn_a, "sub-1", "/topic/room", tx_a);
    broker.subscribe(conn_b, "sub-2", "/topic/other", tx_b);
    broker.publish("/topic/room", &body("hi"), &[]);

    let got = rx_a.try_recv();
    assert!(
        matches!(got, Ok(OutFrame::Frame(_))),
        "subscriber A gets the message"
    );
    assert!(
        rx_b.try_recv().is_err(),
        "subscriber B on another topic gets nothing"
    );
}

#[tokio::test]
async fn unsubscribe_and_unregister_stop_delivery() {
    let broker = Broker::new();
    let conn = broker.register();
    let (tx, mut rx) = mpsc::channel(4);

    broker.subscribe(conn, "sub-1", "/topic/room", tx);
    broker.unsubscribe(conn, "sub-1");
    broker.publish("/topic/room", &body("hi"), &[]);

    assert!(
        rx.try_recv().is_err(),
        "an unsubscribed connection receives nothing"
    );

    let (tx2, mut rx2) = mpsc::channel(4);
    broker.subscribe(conn, "sub-2", "/topic/room", tx2);
    broker.unregister(conn);
    broker.publish("/topic/room", &body("hi"), &[]);

    assert!(
        rx2.try_recv().is_err(),
        "an unregistered connection receives nothing"
    );
}
