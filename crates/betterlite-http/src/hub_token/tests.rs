#![cfg(test)]

use secrecy::ExposeSecret as _;

use super::{HubToken, HubTokenResolver};

#[tokio::test]
async fn token_follows_huggingface_hub() {
    let settings = betterlite_config::sections::huggingface_hub::Settings {
        endpoint: "https://huggingface.co".to_owned(),
        offline: false,
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: false,
        env_token: None,
        hf_home: std::path::PathBuf::new(),
        hub_cache: std::path::PathBuf::new(),
        token_path: std::path::PathBuf::new(),
        stored_tokens_path: std::path::PathBuf::new(),
        hf_transfer_deprecated: false,
    };
    let resolver = HubTokenResolver { settings };
    let explicit = HubToken::Explicit(secrecy::SecretString::from("given"));
    assert_eq!(
        resolver
            .resolve(&explicit)
            .await
            .unwrap()
            .unwrap()
            .expose_secret(),
        "given"
    );
}
