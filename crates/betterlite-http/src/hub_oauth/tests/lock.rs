use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::time::Duration;

use tokio::time::timeout;

use crate::hub_oauth::file_lock::{LockError, acquire};

#[tokio::test]
async fn lock_file_directory_is_created_and_group_writable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing").join("stored_tokens.lock");
    let lock = acquire(&path, Duration::from_secs(1)).await.unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o664);
    drop(lock);
}

#[tokio::test]
async fn held_lock_times_out_and_is_free_again_after_release() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("stored_tokens.lock");
    let held = acquire(&path, Duration::from_secs(1)).await.unwrap();

    let waiting = timeout(
        Duration::from_secs(5),
        acquire(&path, Duration::from_millis(120)),
    )
    .await
    .unwrap();
    assert!(matches!(waiting, Err(LockError::Timeout { .. })));

    drop(held);
    let again = timeout(
        Duration::from_secs(5),
        acquire(&path, Duration::from_secs(1)),
    )
    .await
    .unwrap();
    again.unwrap();
}
