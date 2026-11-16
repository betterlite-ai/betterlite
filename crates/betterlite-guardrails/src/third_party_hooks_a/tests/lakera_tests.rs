use std::collections::{BTreeMap, BTreeSet};

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::enums::Role;
use betterlite_config::sections::guardrail_identity::item::{GuardrailItem, GuardrailValue};
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::legacy_guardrails::Settings;
use secrecy::SecretString;

use super::support::{captured_body, chat_data, list, message, object, serve, text};
use crate::guardrail_selection::error::GuardrailSelectionError;
use crate::guardrail_selection::gate::GuardrailSelection;
use crate::lakera::LegacyLakeraModerationParams;
use crate::lakera::rejection::LakeraViolation;
use crate::lakera::{LegacyLakeraModeration, ModerationCheck};
use crate::third_party_hooks_a::error::ThirdPartyHookError;
use crate::third_party_hooks_a::lakera;

const LAKERA_OK: &str = r#"{"results":[{"flagged":false}]}"#;
const LAKERA_FLAGGED: &str = r#"{"results":[{"flagged":true}]}"#;

struct FakeGate {
    proceed: bool,
    dynamic: BTreeMap<String, GuardrailValue>,
}

fn fake_gate(proceed: bool) -> FakeGate {
    FakeGate {
        proceed,
        dynamic: BTreeMap::new(),
    }
}

impl GuardrailSelection for FakeGate {
    fn should_run_guardrail(
        &self,
        _data: &BTreeMap<String, GuardrailValue>,
        _event_type: GuardrailEventHooks,
        _suppressed: &BTreeSet<String>,
    ) -> Result<bool, GuardrailSelectionError> {
        Ok(true)
    }

    fn event_hook_is_event_type(&self, _event_type: GuardrailEventHooks) -> bool {
        true
    }

    fn is_requested_guardrail(&self, _requested: &[GuardrailValue]) -> bool {
        true
    }

    fn dynamic_request_body_params(
        &self,
        _data: &BTreeMap<String, GuardrailValue>,
    ) -> Result<BTreeMap<String, GuardrailValue>, GuardrailSelectionError> {
        Ok(self.dynamic.clone())
    }
}

fn lakera_hooks(
    base: &str,
    check: ModerationCheck,
    roles: Option<Vec<Role>>,
    add_function_to_prompt: bool,
) -> lakera::LegacyLakeraHooks {
    let moderation = LegacyLakeraModeration::new(
        LegacyLakeraModerationParams {
            moderation_check: Some(check),
            category_thresholds: None,
            api_base: Some(base.to_owned()),
            api_key: Some(SecretString::from("lakera-key")),
        },
        |_name| String::new(),
        reqwest::Client::default,
    );
    let mut by_name = BTreeMap::new();
    if let Some(roles) = roles {
        by_name.insert(
            "prompt_injection".to_owned(),
            GuardrailItem {
                callbacks: Vec::new(),
                default_on: false,
                logging_only: None,
                guardrail_name: "prompt_injection".to_owned(),
                callback_args: BTreeMap::new(),
                enabled_roles: Some(roles),
            },
        );
    }
    lakera::LegacyLakeraHooks::new(
        moderation,
        &Settings {
            items: Vec::new(),
            by_name,
        },
        add_function_to_prompt,
    )
}

#[tokio::test]
async fn lakera_legacy_in_parallel_skips_only_the_pre_call_hook() {
    let (base, _server, captured) = serve(
        "/v1/prompt_injection",
        vec![(StatusCode::OK, LAKERA_OK.to_owned())],
    )
    .await;
    let hooks = lakera_hooks(&base, ModerationCheck::InParallel, None, false);
    let data = chat_data(&[message("user", "u")]);
    let fake = fake_gate(true);
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    hooks
        .pre_call(&data, lakera::LakeraGate::Legacy, &selection)
        .await
        .expect("skip");
    hooks
        .moderation(&data, lakera::LakeraGate::Legacy, &selection)
        .await
        .expect("run");
    assert_eq!(captured.lock().expect("captured").len(), 1);
}

#[tokio::test]
async fn lakera_legacy_pre_call_skips_only_the_moderation_hook() {
    let (base, _server, captured) = serve(
        "/v1/prompt_injection",
        vec![(StatusCode::OK, LAKERA_OK.to_owned())],
    )
    .await;
    let hooks = lakera_hooks(&base, ModerationCheck::PreCall, None, false);
    let data = chat_data(&[message("user", "u")]);
    let fake = fake_gate(true);
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    hooks
        .moderation(&data, lakera::LakeraGate::Legacy, &selection)
        .await
        .expect("skip");
    hooks
        .pre_call(&data, lakera::LakeraGate::Legacy, &selection)
        .await
        .expect("run");
    assert_eq!(captured.lock().expect("captured").len(), 1);
}

#[tokio::test]
async fn lakera_checked_false_skips_both_hooks() {
    let (base, _server, captured) = serve("/v1/prompt_injection", Vec::new()).await;
    let hooks = lakera_hooks(&base, ModerationCheck::InParallel, None, false);
    let data = chat_data(&[message("user", "u")]);
    let fake = fake_gate(true);
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    hooks
        .pre_call(&data, lakera::LakeraGate::Checked(false), &selection)
        .await
        .expect("skip");
    hooks
        .moderation(&data, lakera::LakeraGate::Checked(false), &selection)
        .await
        .expect("skip");
    assert!(captured.lock().expect("captured").is_empty());
}

#[tokio::test]
async fn lakera_metadata_gate_false_ends_the_check() {
    let (base, _server, captured) = serve("/v1/prompt_injection", Vec::new()).await;
    let hooks = lakera_hooks(&base, ModerationCheck::InParallel, None, false);
    let data = chat_data(&[message("user", "u")]);
    let fake = fake_gate(false);
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    hooks
        .check(&data, &selection)
        .await
        .expect("skipped by metadata");
    assert!(captured.lock().expect("captured").is_empty());
}

#[tokio::test]
async fn lakera_posts_system_user_assistant_in_that_order() {
    let (base, _server, captured) = serve(
        "/v1/prompt_injection",
        vec![(StatusCode::OK, LAKERA_OK.to_owned())],
    )
    .await;
    let hooks = lakera_hooks(&base, ModerationCheck::InParallel, None, false);
    let data = chat_data(&[
        message("assistant", "a"),
        message("user", "u"),
        message("system", "s"),
    ]);
    let fake = fake_gate(true);
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    hooks.check(&data, &selection).await.expect("checked");
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/v1/prompt_injection");
    assert_eq!(
        calls[0].headers.get("authorization").map(String::as_str),
        Some("Bearer lakera-key")
    );
    assert_eq!(
        captured_body(&calls, 0),
        object(&[(
            "input",
            list(&[
                message("system", "s"),
                message("user", "u"),
                message("assistant", "a")
            ])
        )])
    );
}

#[tokio::test]
async fn lakera_joins_tool_call_arguments_into_the_system_message() {
    let (base, _server, captured) = serve(
        "/v1/prompt_injection",
        vec![(StatusCode::OK, LAKERA_OK.to_owned())],
    )
    .await;
    let hooks = lakera_hooks(&base, ModerationCheck::InParallel, None, false);
    let assistant = object(&[
        ("role", text("assistant")),
        ("content", text("call")),
        (
            "tool_calls",
            list(&[
                object(&[
                    ("id", text("1")),
                    (
                        "function",
                        object(&[("name", text("f")), ("arguments", text("{\"a\": 1}"))]),
                    ),
                ]),
                object(&[
                    ("id", text("2")),
                    (
                        "function",
                        object(&[("name", text("g")), ("arguments", text("{\"b\": 2}"))]),
                    ),
                ]),
            ]),
        ),
    ]);
    let data = chat_data(&[message("system", "hello"), assistant]);
    let fake = fake_gate(true);
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    hooks.check(&data, &selection).await.expect("checked");
    let calls = captured.lock().expect("captured");
    assert_eq!(
        captured_body(&calls, 0),
        object(&[(
            "input",
            list(&[
                message("system", "hello Function Input: {\"a\": 1} {\"b\": 2}"),
                message("assistant", "call")
            ])
        )])
    );
}

#[tokio::test]
async fn lakera_drops_the_system_message_when_add_function_to_prompt_is_set() {
    let (base, _server, captured) = serve(
        "/v1/prompt_injection",
        vec![(StatusCode::OK, LAKERA_OK.to_owned())],
    )
    .await;
    let hooks = lakera_hooks(&base, ModerationCheck::InParallel, None, true);
    let data = chat_data(&[message("system", "s"), message("user", "u")]);
    let fake = fake_gate(true);
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    hooks.check(&data, &selection).await.expect("checked");
    let calls = captured.lock().expect("captured");
    assert_eq!(
        captured_body(&calls, 0),
        object(&[("input", list(&[message("user", "u")]))])
    );
}

#[tokio::test]
async fn lakera_keeps_only_the_enabled_roles() {
    let (base, _server, captured) = serve(
        "/v1/prompt_injection",
        vec![(StatusCode::OK, LAKERA_OK.to_owned())],
    )
    .await;
    let hooks = lakera_hooks(
        &base,
        ModerationCheck::InParallel,
        Some(vec![Role::User]),
        false,
    );
    let data = chat_data(&[
        message("system", "s"),
        message("user", "u"),
        message("assistant", "a"),
    ]);
    let fake = fake_gate(true);
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    hooks.check(&data, &selection).await.expect("checked");
    let calls = captured.lock().expect("captured");
    assert_eq!(
        captured_body(&calls, 0),
        object(&[("input", list(&[message("user", "u")]))])
    );
}

#[tokio::test]
async fn lakera_skips_the_call_when_no_enabled_role_has_a_message() {
    let (base, _server, captured) = serve("/v1/prompt_injection", Vec::new()).await;
    let hooks = lakera_hooks(
        &base,
        ModerationCheck::InParallel,
        Some(vec![Role::User]),
        false,
    );
    let data = chat_data(&[message("system", "s")]);
    let fake = fake_gate(true);
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    hooks.check(&data, &selection).await.expect("skipped");
    assert!(captured.lock().expect("captured").is_empty());
}

#[tokio::test]
async fn lakera_joins_an_input_list_with_newlines() {
    let (base, _server, captured) = serve(
        "/v1/prompt_injection",
        vec![(StatusCode::OK, LAKERA_OK.to_owned())],
    )
    .await;
    let hooks = lakera_hooks(&base, ModerationCheck::InParallel, None, false);
    let data = BTreeMap::from([("input".to_owned(), list(&[text("a"), text("b")]))]);
    let fake = fake_gate(true);
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    hooks.check(&data, &selection).await.expect("checked");
    let calls = captured.lock().expect("captured");
    assert_eq!(captured_body(&calls, 0), object(&[("input", text("a\nb"))]));
}

#[tokio::test]
async fn lakera_merges_dynamic_request_body_params_into_every_body() {
    let (base, _server, captured) = serve(
        "/v1/prompt_injection",
        vec![
            (StatusCode::OK, LAKERA_OK.to_owned()),
            (StatusCode::OK, LAKERA_OK.to_owned()),
        ],
    )
    .await;
    let hooks = lakera_hooks(&base, ModerationCheck::InParallel, None, false);
    let fake = FakeGate {
        proceed: true,
        dynamic: BTreeMap::from([("metadata".to_owned(), object(&[("trace", text("t"))]))]),
    };
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    let messages = chat_data(&[message("user", "u")]);
    hooks
        .check(&messages, &selection)
        .await
        .expect("messages body");
    let input = BTreeMap::from([("input".to_owned(), text("hello"))]);
    hooks.check(&input, &selection).await.expect("input body");
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 2);
    assert_eq!(
        captured_body(&calls, 0),
        object(&[
            ("input", list(&[message("user", "u")])),
            ("metadata", object(&[("trace", text("t"))]))
        ])
    );
    assert_eq!(
        captured_body(&calls, 1),
        object(&[
            ("input", text("hello")),
            ("metadata", object(&[("trace", text("t"))]))
        ])
    );
}

#[tokio::test]
async fn lakera_returns_the_flagged_rejection() {
    let (base, _server, _captured) = serve(
        "/v1/prompt_injection",
        vec![(StatusCode::OK, LAKERA_FLAGGED.to_owned())],
    )
    .await;
    let hooks = lakera_hooks(&base, ModerationCheck::InParallel, None, false);
    let data = chat_data(&[message("user", "u")]);
    let fake = fake_gate(true);
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    let error = hooks.check(&data, &selection).await.expect_err("flagged");
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    let ThirdPartyHookError::Rejection { detail, .. } = &error else {
        panic!("unexpected error: {error:?}");
    };
    let GuardrailValue::Object(fields) = detail else {
        panic!("detail is not an object");
    };
    assert_eq!(
        fields.get("error"),
        Some(&text(LakeraViolation::ContentSafetyPolicy.message()))
    );
    assert!(fields.contains_key("lakera_ai_response"));
}

#[tokio::test]
async fn lakera_fails_with_the_upstream_error_body() {
    let (base, _server, _captured) = serve(
        "/v1/prompt_injection",
        vec![(
            StatusCode::INTERNAL_SERVER_ERROR,
            "upstream exploded".to_owned(),
        )],
    )
    .await;
    let hooks = lakera_hooks(&base, ModerationCheck::InParallel, None, false);
    let data = chat_data(&[message("user", "u")]);
    let fake = fake_gate(true);
    let selection = lakera::LakeraSelection {
        should_proceed: fake.proceed,
        dynamic_params: &fake,
    };
    let error = hooks.check(&data, &selection).await.expect_err("failed");
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(error.to_string(), "upstream exploded");
    assert!(matches!(error, ThirdPartyHookError::Failure { .. }));
}
