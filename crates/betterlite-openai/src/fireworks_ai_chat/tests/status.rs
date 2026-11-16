use betterlite_prompts::definitions::error::DefinitionsError;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use serde_json::Value;

use super::super::error::FireworksChatError;
use crate::chat_content::{ChatContentError, MissingFileError};
use crate::fireworks_ai_common::error::{FireworksError, MissingApiKey};

#[test]
fn provider_failures_name_their_status() {
    let decode = || serde_json::from_str::<Value>("x").expect_err("not JSON");
    assert_eq!(
        FireworksChatError::MissingApiKey(MissingApiKey).status_code(),
        None
    );
    assert_eq!(
        FireworksChatError::Response(FireworksError {
            status_code: 502,
            message: String::new(),
            headers: None
        })
        .status_code(),
        Some(502)
    );
    assert_eq!(
        FireworksChatError::Reply { source: decode() }.status_code(),
        Some(500)
    );
    assert_eq!(
        FireworksChatError::MessageShape { source: decode() }.status_code(),
        Some(500)
    );
}

#[test]
fn content_helper_failures_name_their_status() {
    let decode = || serde_json::from_str::<Value>("x").expect_err("not JSON");
    assert_eq!(
        FireworksChatError::Definitions(DefinitionsError::ExpansionBudgetExceeded { limit: 1 })
            .status_code(),
        Some(400)
    );
    assert_eq!(
        FireworksChatError::Content(ChatContentError::MissingFile(MissingFileError {
            provider: "fireworks_ai",
            model: Some(String::from("m"))
        }))
        .status_code(),
        Some(400)
    );
    assert_eq!(
        FireworksChatError::Content(ChatContentError::Fetch(ImageFetchError::Disabled {
            url: String::from("https://img/1.png")
        }))
        .status_code(),
        Some(400)
    );
    assert_eq!(
        FireworksChatError::Content(ChatContentError::MessageShape { source: decode() })
            .status_code(),
        Some(500)
    );
}

#[test]
fn request_field_failures_are_bad_requests() {
    assert_eq!(
        FireworksChatError::Tool { index: 0 }.status_code(),
        Some(400)
    );
    assert_eq!(FireworksChatError::Tools.status_code(), Some(400));
    assert_eq!(FireworksChatError::StreamOptions.status_code(), Some(400));
    assert_eq!(FireworksChatError::FileContent.status_code(), Some(400));
    assert_eq!(
        FireworksChatError::ImageContent {
            model: String::from("m")
        }
        .status_code(),
        Some(400)
    );
}

#[test]
fn model_list_failures_carry_no_status() {
    let decode = || serde_json::from_str::<Value>("x").expect_err("not JSON");
    assert_eq!(FireworksChatError::ModelListCredentials.status_code(), None);
    assert_eq!(FireworksChatError::AccountId.status_code(), None);
    assert_eq!(
        FireworksChatError::ModelListStatus {
            status: 500,
            body: String::new()
        }
        .status_code(),
        None
    );
    assert_eq!(
        FireworksChatError::ModelsDecode { source: decode() }.status_code(),
        None
    );
    assert_eq!(FireworksChatError::ModelsTooLarge.status_code(), None);
}
