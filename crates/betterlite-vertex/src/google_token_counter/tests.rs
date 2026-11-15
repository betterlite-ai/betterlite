#![cfg(test)]

mod support;

use std::sync::Arc;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::{
    ConfiguredCredentials, decode_vertex_credentials,
};
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::{Content, Message};
use secrecy::SecretString;

use crate::credential_sources::getters::VertexParams;
use crate::google_token_counter::counter::{
    CountTokensCall, StudioTokenCounter, TokenCounter as _, VertexTokenCounter,
};
use crate::google_token_counter::deployment::DeploymentParams;
use crate::google_token_counter::error::{CountTokensError, ModelInfoError};
use crate::google_token_counter::model_info::{GeminiModelInfo, VertexModelInfo};
use crate::google_token_counter::reply::CountReply;
use crate::studio_identity::AuthValue;

use support::{
    DEADLINE, RecordingMint, Script, StaticExecutor, refresh, refresh_from, runtime, vertex_count,
};

const VERTEX_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.0-flash:countTokens";
const STUDIO_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:countTokens";
const CONVERTED: &str = r#"{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}"#;
const PARTNER_BODY: &str =
    r#"{"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"hello"}]}"#;
const ALIAS_PARTNER_URL: &str = "https://europe-west1-aiplatform.googleapis.com/v1/projects/alias-project/locations/europe-west1/publishers/anthropic/models/count-tokens:rawPredict";
const ASIA_PARTNER_URL: &str = "https://asia-southeast1-aiplatform.googleapis.com/v1/projects/primary-project/locations/asia-southeast1/publishers/anthropic/models/count-tokens:rawPredict";
const EAST5_PARTNER_URL: &str = "https://us-east5-aiplatform.googleapis.com/v1/projects/primary-project/locations/us-east5/publishers/anthropic/models/count-tokens:rawPredict";
const PRIMARY_PARTNER_URL: &str = "https://asia-southeast1-aiplatform.googleapis.com/v1/projects/primary-project/locations/asia-southeast1/publishers/anthropic/models/count-tokens:rawPredict";

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn message(text: &str) -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text(text.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn params<'call>(
    project: Option<&'call str>,
    location: Option<&'call str>,
    credentials: Option<&'call ConfiguredCredentials>,
) -> VertexParams<'call> {
    VertexParams {
        project,
        ai_project: None,
        location,
        ai_location: None,
        credentials,
        ai_credentials: None,
    }
}

fn deployment<'call>(
    model: &'call str,
    vertex: VertexParams<'call>,
    count_tokens_location: Option<&'call str>,
) -> DeploymentParams<'call> {
    DeploymentParams {
        model: Some(model),
        api_key: None,
        gemini_api_key: None,
        api_base: None,
        timeout: None,
        vertex,
        vertex_count_tokens_location: count_tokens_location,
    }
}

fn call<'call>(
    model_to_use: &'call str,
    messages: &'call [Message],
    contents: Option<&'call Json>,
    options: DeploymentParams<'call>,
) -> CountTokensCall<'call> {
    CountTokensCall {
        model_to_use,
        messages,
        contents,
        deployment: options,
        request_model: model_to_use,
        tools: None,
        system: None,
    }
}

fn vertex_counter() -> VertexTokenCounter {
    VertexTokenCounter {
        settings: VertexSettings::default(),
    }
}

#[tokio::test]
async fn selection_empty_contents_wins() {
    let script = Script::replying(br#"{"totalTokens":5,"tokenizer_used":"google"}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj"), Some("us-central1"), None),
        None,
    );
    let contents = json("[]");
    let messages = [message("hello")];
    let request = call("gemini-2.0-flash", &messages, Some(&contents), options);
    let response = vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap()
        .expect("a reply with totalTokens");
    assert_eq!(response.total_tokens, 5);
    assert_eq!(response.tokenizer_type, "google");
    assert_eq!(response.request_model, "gemini-2.0-flash");
    assert!(!response.error);
    assert!(response.error_message.is_none());
    assert!(response.status_code.is_none());
    assert!(matches!(
        &response.original_response,
        Some(CountReply::Google(reply)) if reply.total_tokens == Some(5)
    ));
    let sent = script.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].url, VERTEX_URL);
    assert_eq!(sent[0].body_json(), json(r#"{"contents":[]}"#));
}

#[tokio::test]
async fn selection_vertex_message_fallback() {
    let script = Script::replying(br#"{"totalTokens":3}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj"), Some("us-central1"), None),
        None,
    );
    let messages = [message("hello")];
    let request = CountTokensCall {
        model_to_use: "gemini-2.0-flash",
        messages: &messages,
        contents: None,
        deployment: options,
        request_model: "request-model",
        tools: None,
        system: None,
    };
    let response = vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap()
        .expect("a converted history counts");
    assert_eq!(response.total_tokens, 3);
    assert_eq!(response.request_model, "request-model");
    assert_eq!(response.model_used, "gemini-2.0-flash");
    assert_eq!(response.tokenizer_type, "");
    assert_eq!(script.sent()[0].body_json(), json(CONVERTED));
}

#[tokio::test]
async fn selection_vertex_null_contents() {
    let script = Script::replying(br#"{"totalTokens":3}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj"), Some("us-central1"), None),
        None,
    );
    let messages = [message("hello")];
    let missing = Json::Null;
    let request = call("gemini-2.0-flash", &messages, Some(&missing), options);
    vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap();
    assert_eq!(script.sent()[0].body_json(), json(CONVERTED));
}

#[tokio::test]
async fn selection_tools_and_system_are_ignored() {
    let script = Script::replying(br#"{"totalTokens":2}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj"), Some("us-central1"), None),
        None,
    );
    let contents = json("[]");
    let tools = json(r#"[{"functionDeclarations":[]}]"#);
    let system = json(r#""be brief""#);
    let request = CountTokensCall {
        model_to_use: "gemini-2.0-flash",
        messages: &[],
        contents: Some(&contents),
        deployment: options,
        request_model: "gemini-2.0-flash",
        tools: Some(&tools),
        system: Some(&system),
    };
    vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap();
    assert_eq!(script.sent()[0].body_json(), json(r#"{"contents":[]}"#));
}

#[tokio::test]
async fn selection_studio_none_contents() {
    let script = Script::replying(br#"{"totalTokens":9,"tokenizer_used":"google"}"#);
    let options = deployment("gemini/gemini-2.0-flash", params(None, None, None), None);
    let messages = [message("hello")];
    let tools = json(r#"[{"functionDeclarations":[]}]"#);
    let system = json(r#""be brief""#);
    let request = CountTokensCall {
        model_to_use: "gemini-2.0-flash",
        messages: &messages,
        contents: None,
        deployment: options,
        request_model: "gemini-2.0-flash",
        tools: Some(&tools),
        system: Some(&system),
    };
    let counter = StudioTokenCounter {
        settings: Settings::default(),
    };
    let response = counter
        .count_tokens(&script, &request, DEADLINE)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 9);
    assert_eq!(response.tokenizer_type, "google");
    let sent = script.sent();
    assert_eq!(sent[0].url, STUDIO_URL);
    assert_eq!(sent[0].body_json(), json(r#"{"contents":null}"#));
}

#[tokio::test]
async fn selection_vertex_missing_total() {
    let script = Script::replying(b"{}");
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj"), Some("us-central1"), None),
        None,
    );
    let contents = json("[]");
    let messages = [];
    let request = call("gemini-2.0-flash", &messages, Some(&contents), options);
    let outcome = vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap();
    assert!(outcome.is_none());
    let null_total = Script::replying(br#"{"totalTokens":null}"#);
    let outcome = vertex_count(&null_total, &mint, &refresh, &catalog, &request)
        .await
        .unwrap();
    assert!(outcome.is_none());
    let overflowing = Script::replying(br#"{"totalTokens":18446744073709551615}"#);
    let error = vertex_count(&overflowing, &mint, &refresh, &catalog, &request)
        .await
        .expect_err("a count past i64::MAX is refused");
    assert!(matches!(error, CountTokensError::Overflow));
}

#[tokio::test]
async fn selection_vertex_deployment_target() {
    let script = Script::replying(br#"{"totalTokens":5}"#);
    let catalog = BundledCatalog::default().get();
    let executor = Arc::new(StaticExecutor::default());
    let refresh = refresh_from(Arc::clone(&executor));
    let mint = RecordingMint::default();
    let credentials = ConfiguredCredentials::Text(SecretString::from("vertex-credentials"));
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj-eu"), Some("europe-west1"), Some(&credentials)),
        None,
    );
    let request = call("gemini-2.0-flash", &[], None, options);
    vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap()
        .expect("a reply with totalTokens");
    let sent = script.sent();
    assert!(
        sent[0]
            .url
            .starts_with("https://europe-west1-aiplatform.googleapis.com/")
    );
    assert!(sent[0].url.contains("/projects/proj-eu/"));
    assert!(sent[0].url.contains("/locations/europe-west1/"));
    assert_eq!(
        executor.seen()[0],
        (
            Some("vertex-credentials".to_owned()),
            Some("proj-eu".to_owned())
        )
    );
}

#[tokio::test]
async fn selection_vertex_settings_fallback() {
    let script = Script::replying(br#"{"totalTokens":6}"#);
    let catalog = BundledCatalog::default().get();
    let executor = Arc::new(StaticExecutor::default());
    let refresh = refresh_from(Arc::clone(&executor));
    let mint = RecordingMint::default();
    let counter = VertexTokenCounter {
        settings: VertexSettings {
            project: Some("settings-project".to_owned()),
            location: Some("europe-west1".to_owned()),
            credentials: None,
            adc: AdcEnvironment::default(),
        },
    };
    let options = deployment("vertex_ai/gemini-2.0-flash", params(None, None, None), None);
    let request = call("gemini-2.0-flash", &[], None, options);
    let live = runtime(&script, &mint, &refresh, &catalog);
    counter
        .count_tokens(&live, &request, DEADLINE)
        .await
        .unwrap()
        .expect("a reply with totalTokens");
    let sent = script.sent();
    assert!(
        sent[0]
            .url
            .starts_with("https://europe-west1-aiplatform.googleapis.com/")
    );
    assert!(sent[0].url.contains("/projects/settings-project/"));
    assert!(sent[0].url.contains("/locations/europe-west1/"));
    assert_eq!(
        executor.seen()[0],
        (None, Some("settings-project".to_owned()))
    );
}

#[tokio::test]
async fn selection_studio_missing_total() {
    let script = Script::replying(b"{}");
    let options = deployment("gemini/gemini-2.0-flash", params(None, None, None), None);
    let messages = [];
    let request = call("gemini-2.0-flash", &messages, None, options);
    let counter = StudioTokenCounter {
        settings: Settings::default(),
    };
    let response = counter
        .count_tokens(&script, &request, DEADLINE)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 0);
    assert_eq!(response.tokenizer_type, "");
    assert!(matches!(
        response.original_response,
        Some(CountReply::Google(_))
    ));
    let null_total = Script::replying(br#"{"totalTokens":null}"#);
    let response = counter
        .count_tokens(&null_total, &request, DEADLINE)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 0);
}

#[tokio::test]
async fn selection_studio_deployment_auth_and_base() {
    let script = Script::replying(br#"{"totalTokens":4}"#);
    let api_key = SecretString::from("deployment-key");
    let gemini_api_key = AuthValue::Key(SecretString::from("gemini-key"));
    let options = DeploymentParams {
        model: Some("gemini/gemini-2.0-flash"),
        api_key: Some(&api_key),
        gemini_api_key: Some(&gemini_api_key),
        api_base: Some("https://studio.example.com"),
        timeout: None,
        vertex: params(None, None, None),
        vertex_count_tokens_location: None,
    };
    let request = call("gemini-2.0-flash", &[], None, options);
    let counter = StudioTokenCounter {
        settings: Settings::default(),
    };
    let response = counter
        .count_tokens(&script, &request, DEADLINE)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 4);
    let sent = script.sent();
    assert_eq!(
        sent[0].url,
        "https://studio.example.com/v1beta/models/gemini-2.0-flash:countTokens"
    );
    assert_eq!(
        sent[0].header("x-goog-api-key").as_deref(),
        Some("deployment-key")
    );
}

#[tokio::test]
async fn selection_studio_gemini_key_fallback() {
    let script = Script::replying(br#"{"totalTokens":2}"#);
    let gemini_api_key = AuthValue::Key(SecretString::from("gemini-key"));
    let options = DeploymentParams {
        model: Some("gemini/gemini-2.0-flash"),
        api_key: None,
        gemini_api_key: Some(&gemini_api_key),
        api_base: None,
        timeout: None,
        vertex: params(None, None, None),
        vertex_count_tokens_location: None,
    };
    let request = call("gemini-2.0-flash", &[], None, options);
    let counter = StudioTokenCounter {
        settings: Settings::default(),
    };
    let response = counter
        .count_tokens(&script, &request, DEADLINE)
        .await
        .unwrap();
    assert_eq!(response.total_tokens, 2);
    let sent = script.sent();
    assert_eq!(sent[0].url, STUDIO_URL);
    assert_eq!(
        sent[0].header("x-goog-api-key").as_deref(),
        Some("gemini-key")
    );
}

#[tokio::test]
async fn selection_partner_aliases() {
    let script =
        Script::replying(br#"{"input_tokens":11,"tokenizer_used":"vertex_ai_partner_models"}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let empty = ConfiguredCredentials::Text(SecretString::from(""));
    let alias = ConfiguredCredentials::Text(SecretString::from("alias-credentials"));
    let options = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        VertexParams {
            project: Some(""),
            ai_project: Some("alias-project"),
            location: Some(""),
            ai_location: Some("europe-west1"),
            credentials: Some(&empty),
            ai_credentials: Some(&alias),
        },
        None,
    );
    let messages = [message("hello")];
    let request = call("claude-sonnet-4-6@default", &messages, None, options);
    let response = vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap()
        .expect("a partner count reply");
    assert_eq!(response.total_tokens, 11);
    assert_eq!(response.tokenizer_type, "vertex_ai_partner_models");
    assert!(matches!(
        response.original_response,
        Some(CountReply::Partner(_))
    ));
    assert_eq!(mint.seen()[0].0.as_deref(), Some("alias-credentials"));
    assert_eq!(mint.seen()[0].1.as_deref(), Some("alias-project"));
    let sent = script.sent();
    assert_eq!(sent[0].url, ALIAS_PARTNER_URL);
    assert_eq!(sent[0].body_json(), json(PARTNER_BODY));
}

#[tokio::test]
async fn selection_partner_empty_messages() {
    let script = Script::replying(br#"{"input_tokens":0}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let options = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        params(Some("proj"), Some("us-east5"), None),
        None,
    );
    let tools = json(r#"[{"functionDeclarations":[]}]"#);
    let system = json(r#""be brief""#);
    let messages: [Message; 0] = [];
    let request = CountTokensCall {
        model_to_use: "claude-sonnet-4-6@default",
        messages: &messages,
        contents: None,
        deployment: options,
        request_model: "claude-sonnet-4-6@default",
        tools: Some(&tools),
        system: Some(&system),
    };
    let response = vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap()
        .expect("a partner count reply");
    assert_eq!(response.total_tokens, 0);
    assert_eq!(
        script.sent()[0].body_json(),
        json(r#"{"model":"claude-sonnet-4-6","messages":[]}"#)
    );
}

#[tokio::test]
async fn selection_partner_primary_spelling_wins() {
    let script = Script::replying(br#"{"input_tokens":1}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let primary = ConfiguredCredentials::Text(SecretString::from("primary-credentials"));
    let alias = ConfiguredCredentials::Text(SecretString::from("alias-credentials"));
    let options = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        VertexParams {
            project: Some("primary-project"),
            ai_project: Some("alias-project"),
            location: Some("asia-southeast1"),
            ai_location: Some("europe-west1"),
            credentials: Some(&primary),
            ai_credentials: Some(&alias),
        },
        None,
    );
    let messages = [message("hello")];
    let request = call("claude-sonnet-4-6@default", &messages, None, options);
    vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap();
    assert_eq!(script.sent()[0].url, PRIMARY_PARTNER_URL);
    assert_eq!(mint.seen()[0].0.as_deref(), Some("primary-credentials"));
    assert_eq!(mint.seen()[0].1.as_deref(), Some("primary-project"));
}

#[tokio::test]
async fn selection_partner_count_tokens_location_wins() {
    let script = Script::replying(br#"{"input_tokens":1}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let messages = [message("hello")];
    let pinned = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        params(Some("primary-project"), Some("us-east5"), None),
        Some("asia-southeast1"),
    );
    let pinned_call = call("claude-sonnet-4-6@default", &messages, None, pinned);
    vertex_count(&script, &mint, &refresh, &catalog, &pinned_call)
        .await
        .unwrap();
    let plain = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        params(Some("primary-project"), Some("us-east5"), None),
        None,
    );
    let plain_call = call("claude-sonnet-4-6@default", &messages, None, plain);
    vertex_count(&script, &mint, &refresh, &catalog, &plain_call)
        .await
        .unwrap();
    let sent = script.sent();
    assert_eq!(sent[0].url, ASIA_PARTNER_URL);
    assert_eq!(sent[1].url, EAST5_PARTNER_URL);
}

#[tokio::test]
async fn selection_partner_api_base() {
    let script = Script::replying(br#"{"input_tokens":3}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let mut options = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        params(Some("proj"), Some("us-east5"), None),
        None,
    );
    options.api_base = Some("https://partner.example.com");
    let request = call("claude-sonnet-4-6@default", &[], None, options);
    vertex_count(&script, &mint, &refresh, &catalog, &request)
        .await
        .unwrap()
        .expect("a partner count reply");
    assert_eq!(
        script.sent()[0].url,
        "https://partner.example.com/v1/projects/proj/locations/us-east5/publishers/anthropic/models/count-tokens:rawPredict"
    );
}

#[tokio::test]
async fn selection_partner_settings_fallback() {
    let script = Script::replying(br#"{"input_tokens":3}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let counter = VertexTokenCounter {
        settings: VertexSettings {
            project: Some("settings-project".to_owned()),
            location: Some("europe-west1".to_owned()),
            credentials: None,
            adc: AdcEnvironment::default(),
        },
    };
    let options = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        params(None, None, None),
        None,
    );
    let request = call("claude-sonnet-4-6@default", &[], None, options);
    let live = runtime(&script, &mint, &refresh, &catalog);
    counter
        .count_tokens(&live, &request, DEADLINE)
        .await
        .unwrap()
        .expect("a partner count reply");
    assert_eq!(mint.seen()[0], (None, Some("settings-project".to_owned())));
    assert_eq!(
        script.sent()[0].url,
        "https://europe-west1-aiplatform.googleapis.com/v1/projects/settings-project/locations/europe-west1/publishers/anthropic/models/count-tokens:rawPredict"
    );
}

#[tokio::test]
async fn deployment_isolation() {
    let script = Script::replying(br#"{"totalTokens":4}"#);
    let catalog = BundledCatalog::default().get();
    let refresh = refresh();
    let mint = RecordingMint::default();
    let credentials = ConfiguredCredentials::Text(SecretString::from("credentials"));
    let options = deployment(
        "vertex_ai/gemini-2.0-flash",
        params(Some("proj"), Some("us-central1"), Some(&credentials)),
        Some("asia-southeast1"),
    );
    let messages = [];
    let first = call("gemini-2.0-flash", &messages, None, options);
    let second = call("gemini-2.5-pro", &messages, None, options);
    vertex_count(&script, &mint, &refresh, &catalog, &first)
        .await
        .unwrap();
    vertex_count(&script, &mint, &refresh, &catalog, &second)
        .await
        .unwrap();
    assert_eq!(options.model, Some("vertex_ai/gemini-2.0-flash"));
    assert_eq!(options.vertex.project, Some("proj"));
    assert_eq!(
        options.vertex_count_tokens_location,
        Some("asia-southeast1")
    );
    assert_eq!(
        options
            .vertex
            .credentials
            .and_then(ConfiguredCredentials::as_text),
        Some("credentials")
    );
    let sent = script.sent();
    assert!(sent[0].url.contains("/models/gemini-2.0-flash:countTokens"));
    assert!(sent[1].url.contains("/models/gemini-2.5-pro:countTokens"));
    assert!(sent[1].url.contains("/projects/proj/"));
    assert!(sent[1].url.contains("/locations/us-central1/"));
}

#[test]
fn selection_provider_predicates() {
    let vertex = vertex_counter();
    let studio = StudioTokenCounter {
        settings: Settings::default(),
    };
    assert!(vertex.should_use_token_counting_api(Some("vertex_ai")));
    assert!(!vertex.should_use_token_counting_api(Some("vertex_ai_beta")));
    assert!(!vertex.should_use_token_counting_api(Some("gemini")));
    assert!(!vertex.should_use_token_counting_api(None));
    assert!(studio.should_use_token_counting_api(Some("gemini")));
    assert!(!studio.should_use_token_counting_api(Some("gemini/")));
    assert!(!studio.should_use_token_counting_api(Some("vertex_ai")));
    assert!(!studio.should_use_token_counting_api(None));
}

#[test]
fn selection_location_alias_precedence() {
    let pinned = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        params(Some("proj"), Some("us-east5"), None),
        Some("asia-southeast1"),
    );
    assert_eq!(pinned.explicit_location(), Some("asia-southeast1"));
    let plain = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        params(Some("proj"), Some("us-east5"), None),
        Some(""),
    );
    assert_eq!(plain.explicit_location(), Some("us-east5"));
    let aliased = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        VertexParams {
            project: Some("proj"),
            ai_project: None,
            location: Some(""),
            ai_location: Some("europe-west1"),
            credentials: None,
            ai_credentials: None,
        },
        None,
    );
    assert_eq!(aliased.explicit_location(), Some("europe-west1"));
    let missing = deployment("m", params(None, None, None), None);
    assert_eq!(missing.explicit_location(), None);
}

#[test]
fn selection_source_credentials_are_named() {
    let document = concat!(
        r#"{"type":"service_account","project_id":"proj","#,
        r#""private_key":"-----BEGIN PRIVATE KEY-----\nkey\n-----END PRIVATE KEY-----\n","#,
        r#""client_email":"service@example.com"}"#
    );
    let source =
        ConfiguredCredentials::Source(decode_vertex_credentials(document.as_bytes()).unwrap());
    let text = ConfiguredCredentials::Text(SecretString::from("alias-credentials"));
    let options = deployment(
        "vertex_ai/claude-sonnet-4-6@default",
        VertexParams {
            project: Some("proj"),
            ai_project: None,
            location: Some("us-east5"),
            ai_location: None,
            credentials: Some(&source),
            ai_credentials: Some(&text),
        },
        None,
    );
    assert!(matches!(
        options.explicit_credentials(),
        Some(ConfiguredCredentials::Source(_))
    ));
}

#[test]
fn selection_model_info_stubs_stay_unsupported() {
    let vertex = VertexModelInfo {
        settings: VertexSettings::default(),
    };
    assert!(matches!(
        vertex.validate_environment(),
        Err(ModelInfoError::NotSupported)
    ));
    assert!(matches!(
        vertex.get_models(),
        Err(ModelInfoError::NotSupported)
    ));
    assert!(matches!(
        VertexModelInfo::get_api_key(),
        Err(ModelInfoError::NotSupported)
    ));
    assert!(matches!(
        VertexModelInfo::get_api_base(),
        Err(ModelInfoError::NotSupported)
    ));
    assert!(matches!(
        VertexModelInfo::get_base_model("gemini-2.0-flash"),
        Err(ModelInfoError::NotSupported)
    ));
    assert_eq!(
        ModelInfoError::NotSupported.to_string(),
        "Vertex AI models are not supported yet"
    );
    let studio = GeminiModelInfo {
        settings: Settings::default(),
    };
    assert_eq!(
        studio.get_token_counter().kind(),
        "GoogleAIStudioTokenCounter"
    );
    assert_eq!(vertex.get_token_counter().kind(), "VertexAITokenCounter");
}
