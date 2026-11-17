#![cfg(test)]

use std::sync::Arc;

use betterlite_logging::logging_privacy::{CallbackMetadataError, PrivacyContext};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, MessageContent, Role};
use betterlite_router::semantic_router::service::{
    AutoRouter, AutoRouterSettings, CallerContext, RouteDecision, RouteEmbedder,
};
use indexmap::IndexMap;
use serde_json::Number;

use super::{CallerRequest, caller_context, decide, proxy_server_request};

struct FakeEmbedder;

impl RouteEmbedder for FakeEmbedder {
    type Error = std::io::Error;

    fn embed_documents(
        &self,
        _model: &str,
        texts: Vec<String>,
    ) -> impl Future<Output = Result<Vec<Vec<f64>>, Self::Error>> + Send {
        async move { Ok(texts.iter().map(|_text| vec![1.0, 0.0]).collect()) }
    }

    fn embed_query(
        &self,
        _model: &str,
        _text: String,
        _raw_text: &str,
        _caller: &CallerContext,
    ) -> impl Future<Output = Result<Vec<Vec<f64>>, Self::Error>> + Send {
        async move { Ok(vec![vec![1.0, 0.0]]) }
    }
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: &[(&str, Json)]) -> Json {
    Json::Object(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

fn user_message(text: &str) -> ChatMessage {
    ChatMessage {
        role: Role::User,
        content: Some(MessageContent::Text(text.to_owned())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

fn requested<'a>(metadata: Option<&'a Json>) -> CallerRequest<'a> {
    CallerRequest {
        metadata,
        litellm_metadata: None,
        litellm_params_metadata: None,
        litellm_session_id: None,
        litellm_trace_id: None,
        turn_off_message_logging: None,
    }
}

fn flag_of(request: &CallerRequest<'_>) -> Option<bool> {
    caller_context(request, PrivacyContext::default())
        .expect("the metadata is usable")
        .turn_off_message_logging
}

#[test]
fn the_caller_identity_is_forwarded_with_the_classifier_origin() {
    let auth = object(&[
        ("team_id", text("team-1")),
        ("budget_reservation", text("reservation")),
    ]);
    let metadata = object(&[
        ("user_api_key_team_id", text("team-1")),
        ("user_api_key_budget_reservation", text("reservation")),
        ("user_api_key_auth", auth),
    ]);
    let litellm_metadata = object(&[("user_api_key", text("key-1"))]);
    let request = CallerRequest {
        litellm_metadata: Some(&litellm_metadata),
        ..requested(Some(&metadata))
    };
    let context =
        caller_context(&request, PrivacyContext::default()).expect("the metadata is usable");
    assert_eq!(
        context.metadata.get("user_api_key_team_id"),
        Some(&text("team-1"))
    );
    assert_eq!(
        context.metadata.get("internal_call_origin"),
        Some(&text("autorouter_classifier"))
    );
    assert!(
        !context
            .metadata
            .contains_key("user_api_key_budget_reservation")
    );
    assert_eq!(
        context.metadata.get("user_api_key_auth"),
        Some(&object(&[("team_id", text("team-1"))]))
    );
    assert_eq!(
        context.litellm_metadata.get("user_api_key"),
        Some(&text("key-1"))
    );
    assert_eq!(
        context.litellm_metadata.get("internal_call_origin"),
        Some(&text("autorouter_classifier"))
    );
}

#[test]
fn an_empty_metadata_is_not_stamped_with_the_origin() {
    let empty = object(&[]);
    let present = caller_context(&requested(Some(&empty)), PrivacyContext::default())
        .expect("empty metadata is usable");
    assert!(present.metadata.is_empty());
    let absent = caller_context(&requested(None), PrivacyContext::default())
        .expect("absent metadata is usable");
    assert!(absent.metadata.is_empty());
    assert!(absent.litellm_metadata.is_empty());
    let null = Json::Null;
    let nulled = caller_context(&requested(Some(&null)), PrivacyContext::default())
        .expect("a null metadata takes the default");
    assert!(nulled.metadata.is_empty());
}

#[test]
fn the_flag_comes_from_the_first_slot_that_sets_it() {
    let top_level = Json::Bool(true);
    let metadata = object(&[("turn_off_message_logging", Json::Bool(false))]);
    let litellm_metadata = object(&[("turn_off_message_logging", Json::Bool(true))]);
    let params_metadata = object(&[("turn_off_message_logging", Json::Bool(false))]);
    let direct = CallerRequest {
        turn_off_message_logging: Some(&top_level),
        ..requested(Some(&metadata))
    };
    assert_eq!(flag_of(&direct), Some(true));
    let in_params = CallerRequest {
        litellm_params_metadata: Some(&params_metadata),
        ..requested(Some(&metadata))
    };
    assert_eq!(flag_of(&in_params), Some(false));
    let in_litellm_metadata = CallerRequest {
        litellm_metadata: Some(&litellm_metadata),
        ..requested(Some(&metadata))
    };
    assert_eq!(flag_of(&in_litellm_metadata), Some(true));
    assert_eq!(flag_of(&requested(Some(&metadata))), Some(false));
    assert_eq!(flag_of(&requested(None)), None);
}

#[test]
fn the_privacy_context_forces_the_flag_on() {
    let context = caller_context(
        &requested(None),
        PrivacyContext {
            message_logging_disabled: true,
        },
    )
    .expect("absent metadata is usable");
    assert_eq!(context.turn_off_message_logging, Some(true));
}

#[test]
fn an_environment_reference_in_a_callback_param_is_rejected() {
    let metadata = object(&[("langfuse_public_key", text("os.environ/LANGFUSE_KEY"))]);
    let error = caller_context(&requested(Some(&metadata)), PrivacyContext::default())
        .expect_err("an env reference is not resolved server-side");
    assert!(matches!(
        error,
        CallbackMetadataError::EnvironmentReference { field } if field == "langfuse_public_key"
    ));
}

#[test]
fn session_ids_reach_the_sub_call_only_as_text() {
    let session = text("session-1");
    let trace = Json::Number(Number::from(7));
    let request = CallerRequest {
        litellm_session_id: Some(&session),
        litellm_trace_id: Some(&trace),
        ..requested(None)
    };
    let context =
        caller_context(&request, PrivacyContext::default()).expect("the request is usable");
    assert_eq!(context.litellm_session_id.as_deref(), Some("session-1"));
    assert_eq!(context.litellm_trace_id, None);
}

#[test]
fn a_metadata_that_is_not_an_object_is_rejected() {
    let metadata = text("not an object");
    let error = caller_context(&requested(Some(&metadata)), PrivacyContext::default())
        .expect_err("a string is not a metadata map");
    assert!(matches!(
        error,
        CallbackMetadataError::InvalidField { field } if field == "metadata"
    ));
}

#[test]
fn a_trusted_field_does_not_block_the_flag_and_keeps_the_origin_ours() {
    let metadata = object(&[
        ("internal_call_origin", text("shadow_eval_judge")),
        ("turn_off_message_logging", Json::Bool(true)),
    ]);
    let context = caller_context(&requested(Some(&metadata)), PrivacyContext::default())
        .expect("a stamped origin is ignored, not rejected");
    assert_eq!(context.turn_off_message_logging, Some(true));
    assert_eq!(
        context.metadata.get("internal_call_origin"),
        Some(&text("autorouter_classifier"))
    );
}

#[test]
fn a_flag_of_the_wrong_type_names_the_field() {
    let flag = Json::Number(Number::from(1));
    let request = CallerRequest {
        turn_off_message_logging: Some(&flag),
        ..requested(None)
    };
    let error =
        caller_context(&request, PrivacyContext::default()).expect_err("a number is not a flag");
    assert!(matches!(
        error,
        CallbackMetadataError::InvalidField { field } if field == "turn_off_message_logging"
    ));
}

#[test]
fn the_proxy_body_carries_the_whole_text() {
    let body = proxy_server_request("openai/text-embedding-3-small", "aé中🙂");
    assert_eq!(
        body,
        object(&[(
            "body",
            object(&[
                ("model", text("openai/text-embedding-3-small")),
                ("input", Json::List(vec![text("aé中🙂")])),
            ]),
        )])
    );
}

#[tokio::test]
async fn usable_metadata_routes_on_the_embedding() {
    let metadata = object(&[("user_api_key", text("key-1"))]);
    let router = router();
    let messages = vec![user_message("ask")];
    let decision = decide(
        &router,
        Some(&messages),
        &requested(Some(&metadata)),
        PrivacyContext::default(),
    )
    .await
    .expect("the build succeeds");
    assert_eq!(
        decision,
        Some(RouteDecision {
            route: Some("code".to_owned()),
            model: "code".to_owned()
        })
    );
}

#[tokio::test]
async fn unusable_metadata_falls_back_to_the_default_model() {
    let metadata = text("not an object");
    let router = router();
    let messages = vec![user_message("ask")];
    let decision = decide(
        &router,
        Some(&messages),
        &requested(Some(&metadata)),
        PrivacyContext::default(),
    )
    .await
    .expect("the build still succeeds");
    assert_eq!(
        decision,
        Some(RouteDecision {
            route: None,
            model: "fallback".to_owned()
        })
    );
}

fn router() -> AutoRouter<FakeEmbedder> {
    AutoRouter::new(
        AutoRouterSettings {
            config_path: None,
            config_json: Some(
                r#"{"routes": [{"name": "code", "utterances": ["ask"]}]}"#.to_owned(),
            ),
            default_model: "fallback".to_owned(),
            embedding_model: "fake-embed".to_owned(),
            max_input_chars: None,
        },
        Arc::new(FakeEmbedder),
    )
    .expect("the route config parses")
}
