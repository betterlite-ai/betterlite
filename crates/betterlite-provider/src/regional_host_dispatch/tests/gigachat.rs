use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use betterlite_config::sections::user_url_policy::Settings as UserUrlPolicy;
use betterlite_http::gigachat_tokens::tokens::GigachatTokens;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::gigachat_chat::config::{GigachatBearer, GigachatImageUploader};
use betterlite_openai::gigachat_files::{GigachatFileUploader, GigachatFilesConfig};
use betterlite_openai::gigachat_utils::UuidIds;
use secrecy::ExposeSecret as _;

use crate::regional_host_dispatch::gigachat::{GigachatTokenBearer, chat_config, image_downloader};

use super::support::{FailingTokens, FakeTokens, FakeTransport, proxy_policy, secret};

const IMAGE: &str = "data:image/png;base64,iVBORw0KGgo=";

fn outbound_client() -> (ProxiedClient, UserUrlPolicy) {
    let policy = UserUrlPolicy::default();
    let client =
        ProxiedClient::outbound(&Arc::new(proxy_policy())).expect("the outbound client builds");
    (client, policy)
}

#[tokio::test]
async fn gigachat_token_bearer_returns_the_service_token() {
    let tokens = Arc::new(FakeTokens::new());
    let handle = Arc::clone(&tokens);
    let shared: Arc<dyn GigachatTokens> = handle;
    let bearer = GigachatTokenBearer { tokens: shared };
    let credentials = secret("client-id:client-secret");
    let token = bearer
        .access_token(Some(&credentials))
        .await
        .expect("the bearer answers");
    assert_eq!(token.expose_secret(), "token-1");
    assert_eq!(
        tokens
            .request()
            .credentials
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some(String::from("client-id:client-secret"))
    );
}

#[tokio::test]
async fn gigachat_token_bearer_maps_a_missing_credentials_error() {
    let bearer = GigachatTokenBearer {
        tokens: Arc::new(FailingTokens),
    };
    let error = bearer
        .access_token(None)
        .await
        .expect_err("the failing service gives no token");
    assert_eq!(error.status_code, 401);
    assert_eq!(
        error.message,
        "GigaChat credentials are not set: fill in GIGACHAT_CREDENTIALS or GIGACHAT_API_KEY"
    );
    assert_eq!(error.headers, None);
}

#[tokio::test]
async fn the_uploader_reuses_its_cache_across_requests() {
    let (client, policy) = outbound_client();
    let transport = FakeTransport::new();
    let uploads = transport.counter();
    let bearer: Arc<dyn GigachatBearer> = Arc::new(GigachatTokenBearer {
        tokens: Arc::new(FakeTokens::new()),
    });
    let config = GigachatFilesConfig {
        bearer,
        ids: Arc::new(UuidIds),
        credentials: None,
        api_base: Some(String::from("https://gigachat.example/v1")),
        api_base_env: None,
        downloader: image_downloader(&client, &policy),
    };
    let uploader = GigachatFileUploader::new(transport, config, 8);
    let first = uploader.upload(IMAGE).await;
    let second = uploader.upload(IMAGE).await;
    assert_eq!(first, Some(String::from("file-1")));
    assert_eq!(second, first);
    assert_eq!(uploads.load(Ordering::SeqCst), 1_usize);
}

#[test]
fn the_image_downloader_takes_the_policy_validation_flag() {
    let (client, mut policy) = outbound_client();
    policy.user_url_validation = false;
    let downloader = image_downloader(&client, &policy);
    assert!(!downloader.urls.validation);
    assert!(Arc::ptr_eq(&downloader.client.policy, &client.policy));
}

#[test]
fn the_image_downloader_reads_a_policy_with_validation_on() {
    let (client, policy) = outbound_client();
    let downloader = image_downloader(&client, &policy);
    assert!(downloader.urls.validation);
}

struct RecordingUploader;

#[async_trait]
impl GigachatImageUploader for RecordingUploader {
    async fn upload(&self, _image_url: &str) -> Option<String> {
        Some(String::from("file-from-uploader"))
    }
}

// the chat path gets B8177's config with the token adapter and the uploader the caller keeps alive
#[tokio::test]
async fn chat_config_carries_the_bearer_and_the_uploader() {
    let bearer = GigachatTokenBearer {
        tokens: Arc::new(FakeTokens::new()),
    };
    let uploader = RecordingUploader;
    let config = chat_config(&bearer, &uploader);
    let token = config
        .bearer
        .access_token(None)
        .await
        .expect("the bearer answers");
    assert_eq!(token.expose_secret(), "token-1");
    assert_eq!(
        config.uploader.upload(IMAGE).await,
        Some(String::from("file-from-uploader"))
    );
}
