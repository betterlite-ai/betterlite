use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
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

// filelock opens the lock with O_NOFOLLOW: a symlink is refused before its target can be truncated or rechmodded
#[tokio::test]
async fn symlinked_lock_file_is_refused_and_leaves_its_target_alone() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target");
    fs::write(&target, "keep me").unwrap();
    let mode = fs::metadata(&target).unwrap().permissions().mode();
    let path = directory.path().join("stored_tokens.lock");
    symlink(&target, &path).unwrap();

    let error = acquire(&path, Duration::from_secs(1)).await.unwrap_err();
    assert!(matches!(error, LockError::Open { .. }), "{error:?}");
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep me");
    assert_eq!(fs::metadata(&target).unwrap().permissions().mode(), mode);
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
