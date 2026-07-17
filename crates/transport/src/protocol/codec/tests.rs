use std::time::Duration;

use tokio::io::{AsyncWriteExt, duplex};
use tokio::time::timeout;

use super::{FrameConfig, MessageReader};
use crate::error::Error;
use crate::protocol::{WireMessage, WireRequest};

fn request_frame(id: u64, payload: Vec<u8>) -> Vec<u8> {
    let message = WireMessage::Request(WireRequest {
        id,
        path: "svc.call".to_string(),
        payload,
        streaming_input: false,
    });
    let payload = postcard::to_allocvec(&message).expect("encode request");
    let mut frame = Vec::with_capacity(4 + payload.len());

    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    frame
}

#[tokio::test]
async fn retains_partial_frame_across_cancellation() {
    let frame = request_frame(42, vec![1, 2, 3]);
    let (mut writer, read) = duplex(frame.len());
    let mut reader =
        MessageReader::with_config(read, FrameConfig::new(1024, Duration::from_secs(1)));

    writer
        .write_all(&frame[..6])
        .await
        .expect("write partial frame");

    assert!(
        timeout(Duration::from_millis(20), reader.read_message())
            .await
            .is_err(),
        "the incomplete frame should still be waiting"
    );

    writer.write_all(&frame[6..]).await.expect("finish frame");

    match reader.read_message().await.expect("decode resumed frame") {
        WireMessage::Request(request) => {
            assert_eq!(request.id, 42);
            assert_eq!(request.path, "svc.call");
            assert_eq!(request.payload, vec![1, 2, 3]);
        }
        _ => panic!("expected request"),
    }
}

#[tokio::test]
async fn allocates_only_for_payload_bytes_received() {
    let frame = request_frame(7, vec![0; 32 * 1024]);
    let declared_len = frame.len() - 4;
    let (mut writer, read) = duplex(frame.len());
    let mut reader =
        MessageReader::with_config(read, FrameConfig::new(declared_len, Duration::from_secs(1)));

    writer
        .write_all(&frame[..5])
        .await
        .expect("write prefix and one payload byte");

    assert!(
        timeout(Duration::from_millis(20), reader.read_message())
            .await
            .is_err(),
        "the incomplete frame should still be waiting"
    );
    assert_eq!(reader.payload.len(), 1);
    assert!(
        reader.payload.capacity() < declared_len,
        "declared length must not be allocated eagerly"
    );
}

#[tokio::test]
async fn times_out_when_a_frame_stops_making_progress() {
    let frame = request_frame(9, Vec::new());
    let (mut writer, read) = duplex(frame.len());
    let idle_timeout = Duration::from_millis(10);
    let mut reader = MessageReader::with_config(read, FrameConfig::new(1024, idle_timeout));

    writer
        .write_all(&frame[..2])
        .await
        .expect("write partial prefix");

    assert!(matches!(
        reader.read_message().await,
        Err(Error::ReadTimeout { idle_timeout: actual }) if actual == idle_timeout
    ));
}

#[tokio::test]
async fn idle_connection_has_no_deadline_before_frame_starts() {
    let frame = request_frame(10, Vec::new());
    let (mut writer, read) = duplex(frame.len());
    let mut reader =
        MessageReader::with_config(read, FrameConfig::new(1024, Duration::from_millis(10)));

    {
        let pending_read = reader.read_message();
        tokio::pin!(pending_read);
        assert!(
            timeout(Duration::from_millis(30), &mut pending_read)
                .await
                .is_err(),
            "an idle connection must remain open between frames"
        );
    }

    writer
        .write_all(&frame)
        .await
        .expect("write complete frame");
    assert!(matches!(
        reader.read_message().await.expect("decode after idle"),
        WireMessage::Request(request) if request.id == 10
    ));
}

#[tokio::test]
async fn releases_large_payload_capacity_after_decode() {
    let frame = request_frame(11, vec![0; 64 * 1024]);
    let (mut writer, read) = duplex(frame.len());
    let mut reader =
        MessageReader::with_config(read, FrameConfig::new(frame.len(), Duration::from_secs(1)));

    writer.write_all(&frame).await.expect("write large frame");
    reader.read_message().await.expect("decode large frame");

    assert!(reader.payload.is_empty());
    assert!(reader.payload.capacity() <= super::RETAINED_PAYLOAD_CAPACITY);
}

#[tokio::test]
async fn rejects_oversized_frame_before_allocating_payload() {
    let (mut writer, read) = duplex(4);
    let mut reader = MessageReader::with_config(read, FrameConfig::new(8, Duration::from_secs(1)));

    writer
        .write_all(&9_u32.to_le_bytes())
        .await
        .expect("write length prefix");

    assert!(matches!(
        reader.read_message().await,
        Err(Error::FrameTooLarge { len: 9, max: 8 })
    ));
    assert!(reader.payload.is_empty());
}
