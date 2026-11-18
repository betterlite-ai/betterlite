use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let lease = lease_object(manager(None));
    assert!(!lease.redis_available());
    assert_eq!(lease.acquire_lock(JOB).await, None);
    lease.release_lock(JOB).await;
}
