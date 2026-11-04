#![cfg(test)]

use std::path::PathBuf;

use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use tokio::fs;

use betterlite_config::sections::huggingface_hub::{OfflineSetting, Settings, TransferSetting};

use super::{HubToken, HubTokenError, HubTokenResolver};

fn resolver(token_path: PathBuf, env_token: Option<&str>, disabled: bool) -> HubTokenResolver {
    HubTokenResolver {
        settings: Settings {
            endpoint: "https://huggingface.co".to_owned(),
            offline: OfflineSetting::Online,
            etag_timeout: 10,
            download_timeout: 10,
            disable_implicit_token: disabled,
            env_token: env_token.map(SecretString::from),
            hf_home: PathBuf::new(),
            hub_cache: PathBuf::new(),
            token_path,
            stored_tokens_path: PathBuf::new(),
            transfer: TransferSetting::Standard,
        },
    }
}

#[tokio::test]
async fn token_follows_huggingface_hub() {
    let directory = tempfile::tempdir().unwrap();
    let token_path = directory.path().join("token");
    fs::write(&token_path, " file\r\ntoken \n").await.unwrap();
    explicit_and_implicit_tokens(token_path.clone()).await;
    env_token_precedes_file(token_path.clone()).await;
    file_failures_are_reported(token_path, directory.path().to_path_buf()).await;
}

async fn explicit_and_implicit_tokens(token_path: PathBuf) {
    let explicit = HubToken::Explicit(SecretString::from("given"));
    let empty_explicit = HubToken::Explicit(SecretString::from(""));
    let file_resolver = resolver(token_path.clone(), None, false);
    assert_eq!(
        file_resolver
            .resolve(&explicit)
            .await
            .unwrap()
            .unwrap()
            .expose_secret(),
        "given"
    );
    assert_eq!(
        file_resolver
            .resolve(&empty_explicit)
            .await
            .unwrap()
            .unwrap()
            .expose_secret(),
        ""
    );
    assert_eq!(
        file_resolver
            .resolve(&HubToken::Implicit)
            .await
            .unwrap()
            .unwrap()
            .expose_secret(),
        "filetoken"
    );
}

async fn env_token_precedes_file(token_path: PathBuf) {
    let env_resolver = resolver(token_path.clone(), Some("environment"), false);
    assert_eq!(
        env_resolver
            .resolve(&HubToken::Implicit)
            .await
            .unwrap()
            .unwrap()
            .expose_secret(),
        "environment"
    );
    let disabled_resolver = resolver(token_path, Some("environment"), true);
    assert!(
        disabled_resolver
            .resolve(&HubToken::Implicit)
            .await
            .unwrap()
            .is_none()
    );
}

async fn file_failures_are_reported(token_path: PathBuf, directory: PathBuf) {
    fs::write(&token_path, " \r\n ").await.unwrap();
    let file_resolver = resolver(token_path.clone(), None, false);
    assert!(
        file_resolver
            .resolve(&HubToken::Implicit)
            .await
            .unwrap()
            .is_none()
    );
    let missing_resolver = resolver(directory.join("missing"), None, false);
    assert!(
        missing_resolver
            .resolve(&HubToken::Implicit)
            .await
            .unwrap()
            .is_none()
    );

    let directory_resolver = resolver(directory.clone(), None, false);
    assert!(
        matches!(directory_resolver.resolve(&HubToken::Implicit).await, Err(HubTokenError::Read { path, .. }) if path == directory)
    );
    fs::write(&token_path, [0xff, 0xfe]).await.unwrap();
    assert!(
        matches!(file_resolver.resolve(&HubToken::Implicit).await, Err(HubTokenError::Utf8 { path, .. }) if path == token_path)
    );
}
