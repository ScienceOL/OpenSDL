use serde_json::json;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

use super::{Client, Error};

const BROADCAST_REQUEST: [u8; 20] = [
    0x5a, 0x58, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xf1, 1, 0, 0, 0x27, 0x10, 1, 5, 0x24, 13, 10,
];
const VERIFIED_ADDRESS: [u8; 7] = [0, 0xd1, 0, 0, 0, 0x42, 1];
pub(in crate::sc9300) const IDENTITY_PAYLOAD: [u8; 10] = [0, 0xd1, 0, 0, 0, 0x42, 1, 3, 1, 0];

pub(in crate::sc9300) fn response(command: u16, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![
        0x5a,
        0x58,
        0,
        payload.len() as u8,
        0xff,
        0xff,
        0xff,
        0xff,
        0xff,
        0xf1,
        1,
        0,
        0,
        0x27,
        0x11,
    ];
    frame.extend(command.to_be_bytes());
    frame.extend(payload);
    let sum: u32 = frame.iter().map(|&byte| u32::from(byte)).sum();
    frame.extend([(256 - sum % 256) as u8, 13, 10]);
    frame
}

#[tokio::test]
async fn joins_fragmented_frames_ignores_ack_and_corruption_and_routes_status_to_the_controller() {
    let (link, mut instrument) = duplex(512);
    let device = tokio::spawn(async move {
        let mut request = [0; 20];
        instrument.read_exact(&mut request).await.unwrap();
        assert_eq!(request, BROADCAST_REQUEST);

        let mut corrupt = response(0x8105, &IDENTITY_PAYLOAD);
        corrupt[27] ^= 1;
        instrument.write_all(&[0xff, 0x5a, 0x00]).await.unwrap();
        instrument.write_all(&response(0x8105, &[0])).await.unwrap();
        instrument.write_all(&corrupt).await.unwrap();
        let valid = response(0x8105, &IDENTITY_PAYLOAD);
        instrument.write_all(&valid[..11]).await.unwrap();
        tokio::task::yield_now().await;
        instrument.write_all(&valid[11..]).await.unwrap();

        instrument.read_exact(&mut request).await.unwrap();
        assert_eq!(&request[4..11], &VERIFIED_ADDRESS);
        assert_eq!(
            &request[11..],
            &[0, 0, 0x27, 0x11, 0x01, 0x05, 0xfc, 13, 10]
        );
        instrument.write_all(&valid).await.unwrap();

        instrument.read_exact(&mut request).await.unwrap();
        assert_eq!(&request[4..11], &VERIFIED_ADDRESS);
        assert_eq!(
            &request[11..],
            &[0, 0, 0x27, 0x12, 0x00, 0x00, 0x01, 13, 10]
        );
        instrument.write_all(&response(0x8000, &[0])).await.unwrap();
        instrument
            .write_all(&response(0x8000, &[0, 0, 0x27, 0x11, 1, 5, 1, 0x11]))
            .await
            .unwrap();
    });

    let mut client = Client::new(link);
    let observed = client.identify(None).await.unwrap();
    let identity = client.identify(Some(observed)).await.unwrap();
    assert_eq!(
        json!(identity.info()),
        json!({"device_type":"00D1","device_id":"00000042","firmware":"1.3.1.0"})
    );
    assert_eq!(
        json!(client.status(identity).await.unwrap()),
        json!({"command_serial":10001,"command":261,"state":"in_progress","state_code":1,"error_code":17})
    );
    device.await.unwrap();
}

#[tokio::test]
async fn rejects_another_instrument_model_before_sending_a_status_request() {
    let (link, mut instrument) = duplex(512);
    let device = tokio::spawn(async move {
        let mut request = [0; 20];
        instrument.read_exact(&mut request).await.unwrap();
        assert_eq!(request, BROADCAST_REQUEST);
        let mut payload = IDENTITY_PAYLOAD;
        payload[1] = 0xd2;
        instrument
            .write_all(&response(0x8105, &payload))
            .await
            .unwrap();
        assert_eq!(instrument.read(&mut request).await.unwrap(), 0);
    });

    let mut client = Client::new(link);
    assert!(matches!(client.identify(None).await, Err(Error::Identity)));
    drop(client);
    device.await.unwrap();
}
