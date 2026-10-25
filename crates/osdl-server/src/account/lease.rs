use super::identity::now;
use super::{Identity, Principal};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::watch;
type Registry = Mutex<HashMap<Principal, Weak<Lease>>>;
#[derive(Clone, Default, Debug)]
pub(crate) struct Leases(Arc<Registry>);
pub(crate) struct Lease {
    principal: Principal,
    deadline: watch::Sender<u64>,
    registry: Weak<Registry>,
}
impl Leases {
    pub fn acquire(&self, identity: &Identity) -> Arc<Lease> {
        let mut registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let existing = registry.get(&identity.principal).and_then(Weak::upgrade);
        if existing
            .as_ref()
            .is_some_and(|lease| *lease.deadline.borrow() > now())
        {
            return existing.expect("live lease");
        }
        let lease = Arc::new(Lease {
            principal: identity.principal.clone(),
            deadline: watch::channel(identity.expires_at).0,
            registry: Arc::downgrade(&self.0),
        });
        registry.insert(identity.principal.clone(), Arc::downgrade(&lease));
        drop(registry);
        drop(existing);
        lease
    }
    pub fn renew(&self, identity: &Identity) {
        let lease = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&identity.principal)
            .and_then(Weak::upgrade);
        if let Some(lease) = lease {
            lease.deadline.send_if_modified(|expiry| {
                if *expiry > now() && identity.expires_at > *expiry {
                    *expiry = identity.expires_at;
                    true
                } else {
                    false
                }
            });
        }
    }
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
            if registry
                .get(&self.principal)
                .is_some_and(|entry| std::ptr::eq(entry.as_ptr(), self))
            {
                registry.remove(&self.principal);
            }
        }
    }
}
impl Lease {
    /// A deadline observer does not own the authorization lease or its registry entry.
    pub(crate) fn deadline(&self) -> watch::Receiver<u64> {
        self.deadline.subscribe()
    }
    pub async fn expired(&self) {
        let mut deadline = self.deadline.subscribe();
        loop {
            let expiry = *deadline.borrow_and_update();
            let duration = (std::time::UNIX_EPOCH + std::time::Duration::from_secs(expiry))
                .duration_since(std::time::SystemTime::now())
                .unwrap_or_default();
            if duration.is_zero() {
                return;
            }
            tokio::select! {
                biased;
                _ = deadline.changed() => {},
                _ = tokio::time::sleep(duration) => {},
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn identity(seconds: u64) -> Identity {
        Identity {
            principal: Principal {
                device_id: uuid::Uuid::new_v4(),
                device_generation: 1,
                installation_id: uuid::Uuid::new_v4(),
            },
            scope: crate::account::Scope::Read,
            expires_at: super::super::identity::now() + seconds,
        }
    }
    #[tokio::test]
    async fn renewed_stream_lives_then_expires_and_releases_registry() {
        let registry = Leases::default();
        let mut id = identity(2);
        let lease = registry.acquire(&id);
        assert_eq!(registry.len(), 1);
        id.expires_at += 2;
        registry.renew(&id);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(2100), lease.expired())
                .await
                .is_err()
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), lease.expired())
            .await
            .unwrap();
        drop(lease);
        assert_eq!(registry.len(), 0);
    }
    #[tokio::test]
    async fn different_generation_cannot_extend_or_resurrect_expired_stream() {
        let registry = Leases::default();
        let mut id = identity(0);
        let lease = registry.acquire(&id);
        id.expires_at += 300;
        id.principal.device_generation += 1;
        registry.renew(&id);
        tokio::time::timeout(std::time::Duration::from_millis(30), lease.expired())
            .await
            .unwrap();
        id.principal.device_generation -= 1;
        registry.renew(&id);
        tokio::time::timeout(std::time::Duration::from_millis(30), lease.expired())
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn a_different_device_generation_or_installation_cannot_renew_a_live_lease() {
        let registry = Leases::default();
        let id = identity(1);
        let lease = registry.acquire(&id);
        for field in 0..3 {
            let mut other = id.clone();
            other.expires_at += 299;
            match field {
                0 => other.principal.device_generation += 1,
                1 => other.principal.device_id = uuid::Uuid::new_v4(),
                _ => other.principal.installation_id = uuid::Uuid::new_v4(),
            }
            registry.renew(&other);
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), lease.expired())
            .await
            .unwrap();
    }
    #[test]
    fn completed_history_does_not_grow_active_lease_registry() {
        let registry = Leases::default();
        let start = std::time::Instant::now();
        for _ in 0..10000 {
            drop(registry.acquire(&identity(300)));
        }
        assert_eq!(registry.len(), 0);
        let history = start.elapsed();
        let start = std::time::Instant::now();
        let active: Vec<_> = (0..1000)
            .map(|_| registry.acquire(&identity(300)))
            .collect();
        assert_eq!(registry.len(), 1000);
        drop(active);
        assert_eq!(registry.len(), 0);
        println!(
            "PERF completed_10000_leases_us={} active_1000_create_drop_us={} retained_entries={}",
            history.as_micros(),
            start.elapsed().as_micros(),
            registry.len()
        );
    }
}
