use std::fs;

use betterlite_logging::private_json::files::PrivateFiles;

use crate::credential_lifecycle::files::{CredentialFiles as _, ManagedTokenFile};

#[tokio::test]
async fn remove_treats_only_a_missing_token_file_as_gone() {
    let directory = tempfile::tempdir().expect("makes a token directory");
    let missing = ManagedTokenFile {
        files: PrivateFiles::default(),
        path: directory.path().join("missing.json"),
    };
    let unremovable = ManagedTokenFile {
        files: PrivateFiles::default(),
        path: directory.path().join("token.json"),
    };
    fs::create_dir_all(&unremovable.path).expect("makes a directory where the file would be");

    missing.remove().await.expect("a missing file is gone");
    unremovable
        .remove()
        .await
        .expect_err("a directory is not a removed file");
}
