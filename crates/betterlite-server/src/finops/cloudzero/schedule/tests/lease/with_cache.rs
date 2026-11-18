use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let cache = Arc::new(FakeCache::default());
    let lease = lease_object(manager(Some(Arc::clone(&cache))));
    assert!(lease.redis_available());
    assert_eq!(lease.acquire_lock(JOB).await, Some(true));
    assert_eq!(lease.acquire_lock(JOB).await, Some(true));
    lease.release_lock(JOB).await;
    let deleted = cache.deleted.lock().expect("an unsplit lock").clone();
    assert_eq!(deleted, vec![LEASE_KEY.to_owned()]);
}
