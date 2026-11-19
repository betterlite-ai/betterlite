#![cfg(test)]

use std::future::{Future, ready};

use betterlite_openai::copilot::api_key::error::ApiKeyError;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::CopilotMessagesError;
use super::{
    API_VERSION, CUSTOM_LLM_PROVIDER, FILTERS_ANTHROPIC_BETA_HEADERS, HANDLES_WEB_SEARCH_NATIVELY,
    Headers, MESSAGES_PATH, MessagesConfig, MessagesEnvironment, MessagesRequest,
};
use crate::api::copilot_chat::credentials::CopilotCredentials;

enum Outcome {
    Key(&'static str),
    NoToken,
    Refresh,
}

struct Stored {
    outcome: Outcome,
    base: Option<&'static str>,
}

impl CopilotCredentials for Stored {
    fn api_base(&mut self) -> impl Future<Output = Option<String>> + Send {
        ready(self.base.map(String::from))
    }

    fn api_key(
        &mut self,
    ) -> impl Future<Output = Result<Option<SecretString>, ApiKeyError>> + Send {
        ready(match &self.outcome {
            Outcome::Key(text) => Ok(Some(SecretString::from(String::from(*text)))),
            Outcome::NoToken => Ok(None),
            Outcome::Refresh => Err(ApiKeyError::Refresh),
        })
    }
}

fn request<'request>(
    optional_params: &'request IndexMap<String, Json>,
    messages: &'request [Json],
) -> MessagesRequest<'request> {
    MessagesRequest {
        optional_params,
        messages,
    }
}

fn environment(headers: &Headers, name: &str) -> String {
    headers.get(name).cloned().unwrap_or_default()
}

#[test]
fn identity_constants_mark_the_messages_proxy() {
    assert_eq!(<&'static str>::from(CUSTOM_LLM_PROVIDER), "github_copilot");
    assert_eq!(API_VERSION, "2026-06-01");
    assert_eq!(MESSAGES_PATH, "/v1/messages");
    assert_eq!(
        (HANDLES_WEB_SEARCH_NATIVELY, FILTERS_ANTHROPIC_BETA_HEADERS),
        (false, false)
    );
}

#[tokio::test]
async fn validate_environment_adds_the_session_headers_and_forces_the_proxy_values() {
    let optional = IndexMap::new();
    let messages = [Json::Object(IndexMap::new())];
    let mut config = MessagesConfig {
        credentials: Stored {
            outcome: Outcome::Key("stored-token"),
            base: Some("https://session.test/"),
        },
    };
    let caller = Headers::from_iter([
        (
            String::from("openai-intent"),
            String::from("conversation-panel"),
        ),
        (
            String::from("x-github-api-version"),
            String::from("2020-01-01"),
        ),
        (String::from("x-caller"), String::from("kept")),
    ]);

    let MessagesEnvironment { headers, api_base } = config
        .validate_environment(caller, &request(&optional, &messages), "claude-sonnet-4")
        .await
        .expect("the stored token validates");

    assert_eq!(api_base, "https://session.test");
    assert_eq!(
        environment(&headers, "Authorization"),
        "Bearer stored-token"
    );
    assert_eq!(environment(&headers, "x-caller"), "kept");
    assert_eq!(environment(&headers, "openai-intent"), "messages-proxy");
    assert_eq!(
        environment(&headers, "x-interaction-type"),
        "messages-proxy"
    );
    assert_eq!(environment(&headers, "x-github-api-version"), "2026-06-01");
    assert_eq!(environment(&headers, "anthropic-version"), "2023-06-01");
    assert_eq!(environment(&headers, "editor-version"), "vscode/1.95.0");
    assert_eq!(environment(&headers, "content-type"), "application/json");
    assert_eq!(headers.len(), 13);
    assert!(!headers.contains_key("anthropic-beta"));
}

#[tokio::test]
async fn validate_environment_keeps_a_caller_anthropic_version() {
    let optional = IndexMap::new();
    let messages = [Json::Object(IndexMap::new())];
    let mut config = MessagesConfig {
        credentials: Stored {
            outcome: Outcome::Key("stored-token"),
            base: None,
        },
    };
    let caller = Headers::from_iter([(
        String::from("anthropic-version"),
        String::from("2024-01-01"),
    )]);

    let MessagesEnvironment { headers, api_base } = config
        .validate_environment(caller, &request(&optional, &messages), "claude-sonnet-4")
        .await
        .expect("the stored token validates");

    assert_eq!(environment(&headers, "anthropic-version"), "2024-01-01");
    assert_eq!(api_base, "https://api.githubcopilot.com");
}

#[tokio::test]
async fn validate_environment_renders_a_missing_token_as_none() {
    let optional = IndexMap::new();
    let messages = [Json::Object(IndexMap::new())];
    let mut config = MessagesConfig {
        credentials: Stored {
            outcome: Outcome::NoToken,
            base: None,
        },
    };

    let MessagesEnvironment { headers, .. } = config
        .validate_environment(
            Headers::new(),
            &request(&optional, &messages),
            "claude-sonnet-4",
        )
        .await
        .expect("a missing token is not an error here");

    assert_eq!(environment(&headers, "Authorization"), "Bearer None");
}

#[tokio::test]
async fn validate_environment_translates_a_key_error() {
    let optional = IndexMap::new();
    let messages = [Json::Object(IndexMap::new())];
    let mut config = MessagesConfig {
        credentials: Stored {
            outcome: Outcome::Refresh,
            base: None,
        },
    };

    let error = config
        .validate_environment(
            Headers::new(),
            &request(&optional, &messages),
            "claude-sonnet-4",
        )
        .await
        .err()
        .expect("a key error is an authentication error");

    assert!(matches!(
        &error,
        CopilotMessagesError::Authentication { .. }
    ));
    assert_eq!(error.status(), Some(401));
}

#[tokio::test]
async fn validate_environment_injects_the_requested_betas() {
    let optional = IndexMap::from([(String::from("speed"), Json::Text(String::from("fast")))]);
    let messages = [Json::Object(IndexMap::new())];
    let mut config = MessagesConfig {
        credentials: Stored {
            outcome: Outcome::Key("stored-token"),
            base: None,
        },
    };
    let caller = Headers::from_iter([(
        String::from("anthropic-beta"),
        String::from("existing-beta"),
    )]);

    let MessagesEnvironment { headers, .. } = config
        .validate_environment(caller, &request(&optional, &messages), "claude-sonnet-4")
        .await
        .expect("the stored token validates");

    assert_eq!(
        environment(&headers, "anthropic-beta"),
        "existing-beta,fast-mode-2026-02-01"
    );
}

#[tokio::test]
async fn validate_environment_reports_a_malformed_tools_value() {
    let optional = IndexMap::from([(String::from("tools"), Json::Text(String::from("tools")))]);
    let messages = [Json::Object(IndexMap::new())];
    let mut config = MessagesConfig {
        credentials: Stored {
            outcome: Outcome::Key("stored-token"),
            base: None,
        },
    };

    let error = config
        .validate_environment(
            Headers::new(),
            &request(&optional, &messages),
            "claude-sonnet-4",
        )
        .await
        .err()
        .expect("tools must be a list");

    assert!(matches!(&error, CopilotMessagesError::Beta(_)));
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn url_appends_the_messages_path_once() {
    let mut config = MessagesConfig {
        credentials: Stored {
            outcome: Outcome::Key("stored-token"),
            base: Some("https://session.test//"),
        },
    };

    assert_eq!(
        config.get_complete_url(Some("https://trusted.test/")).await,
        "https://trusted.test/v1/messages"
    );
    assert_eq!(
        config
            .get_complete_url(Some("https://trusted.test/v1/messages/"))
            .await,
        "https://trusted.test/v1/messages"
    );
    assert_eq!(
        config.get_complete_url(Some("")).await,
        "https://session.test/v1/messages"
    );
    assert_eq!(
        config.get_complete_url(None).await,
        "https://session.test/v1/messages"
    );
}

#[tokio::test]
async fn url_falls_back_to_the_public_endpoint() {
    let mut config = MessagesConfig {
        credentials: Stored {
            outcome: Outcome::Key("stored-token"),
            base: None,
        },
    };

    assert_eq!(
        config.get_complete_url(None).await,
        "https://api.githubcopilot.com/v1/messages"
    );
}
