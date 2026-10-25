use super::*;
use crate::authorization::lease::Lease;
use std::{future::Future, task::Poll};

fn request() -> pb::WaitForDeviceRequest {
    pb::WaitForDeviceRequest {
        selector: Some(pb::wait_for_device_request::Selector::DeviceId(
            "target".into(),
        )),
        timeout_ms: u32::MAX,
    }
}
fn event() -> OsdlEvent {
    OsdlEvent::DeviceOnline(osdl_core::Device {
        id: "target".into(),
        transport_id: "test".into(),
        device_type: "test".into(),
        adapter: String::new(),
        description: String::new(),
        online: true,
        properties: Default::default(),
        properties_timestamp_ms: 0,
        actions: vec![],
        role: None,
    })
}

#[tokio::test]
async fn revoke_drops_subscription_before_ack_and_wins_ready_event_race() {
    for ready_event in [false, true] {
        let engine = osdl_core::OsdlEngine::new(Default::default(), vec![]);
        let handle = engine.handle();
        let (events, subscription) = broadcast::channel(8);
        let lease = Lease::grant(osdl_auth::now() + 300);
        let mut waiting = Box::pin(wait(
            &handle,
            request(),
            subscription,
            Some(lease.resource().unwrap()),
        ));
        std::future::poll_fn(|cx| {
            assert!(waiting.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert_eq!(events.receiver_count(), 1);
        assert!(!lease.is_drained());
        if ready_event {
            events.send(event()).unwrap();
        }
        lease.revoke();
        let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(waiting, async {
                lease.drained().await;
                assert_eq!(
                    events.receiver_count(),
                    0,
                    "subscription must be gone before revoke acknowledges"
                );
            })
        })
        .await
        .unwrap();
        assert_eq!(
            result
                .unwrap_err()
                .metadata()
                .get("opensdl-auth-error")
                .unwrap(),
            "grant_revoked"
        );
    }
}

#[tokio::test]
async fn expiry_drops_absent_device_subscription_and_wins_ready_event_race() {
    for ready_event in [false, true] {
        let engine = osdl_core::OsdlEngine::new(Default::default(), vec![]);
        let handle = engine.handle();
        let (events, subscription) = broadcast::channel(8);
        let lease = Lease::grant(osdl_auth::now() + 1);
        let mut waiting = Box::pin(wait(
            &handle,
            request(),
            subscription,
            Some(lease.resource().unwrap()),
        ));
        std::future::poll_fn(|cx| {
            assert!(waiting.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        if ready_event {
            events.send(event()).unwrap();
        }
        lease.expired().await;
        let error = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error.metadata().get("opensdl-auth-error").unwrap(),
            "grant_expired"
        );
        assert!(lease.is_drained());
        assert_eq!(events.receiver_count(), 0);
    }
}

#[tokio::test]
async fn local_wait_still_returns_a_matching_device_and_drops_subscription() {
    let engine = osdl_core::OsdlEngine::new(Default::default(), vec![]);
    let (events, subscription) = broadcast::channel(8);
    events.send(event()).unwrap();
    let device = wait(&engine.handle(), request(), subscription, None)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(device.id, "target");
    assert_eq!(events.receiver_count(), 0);
}
