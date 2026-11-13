use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{build, encoded, init, number, object, params, text};
use crate::logging_state::LoggingInit;
use crate::logging_state::LoggingState;
use crate::logging_state::types::Messages;
use crate::logging_state::updates::{
    CallUpdate, update_environment_variables, update_from_kwargs, update_messages,
};

enum Scenario {
    Environment,
    ModelKept,
    UserCleared,
    PricingNull,
    PricingAbsent,
    ProviderWrongType,
    ProviderNull,
    FromKwargs,
    FromKwargsMetadataOnly,
    FromKwargsEmptyCallerMetadata,
    FromKwargsNonObjectMetadata,
    MessagesUpdated,
}

fn summary(state: &LoggingState, keys: &[&str]) -> String {
    let mut values = IndexMap::new();
    for key in keys {
        let value = state
            .model_call_details
            .get(*key)
            .cloned()
            .unwrap_or_else(|| Json::Text("<missing>".to_owned()));
        values.insert((*key).to_owned(), value);
    }
    encoded(&Json::Object(values))
}

fn environment(update: CallUpdate, litellm_params: &IndexMap<String, Json>) -> String {
    let mut state = build(LoggingInit {
        messages: Some(Messages::Text("hi".to_owned())),
        callback_dynamic_params: params(&[("langfuse_host", text("h"))]),
        ..init()
    });
    update_environment_variables(&mut state, litellm_params, update)
        .expect("the fixture update succeeds");
    format!(
        "{}/{}/{}/{}/{}/{}/{}/{}",
        state.model,
        encoded(&state.user),
        state.litellm_request_debug,
        encoded(&state.stream_options),
        state.cost.custom_pricing,
        encoded(&state.custom_llm_provider),
        encoded(&state.litellm_params),
        summary(
            &state,
            &[
                "model",
                "messages",
                "optional_params",
                "litellm_params",
                "start_time",
                "stream",
                "user",
                "call_type",
                "custom_llm_provider",
                "stream_options",
                "standard_callback_dynamic_params",
            ],
        )
    )
}

fn call_update() -> CallUpdate {
    CallUpdate {
        model: Some("gpt-4.1".to_owned()),
        user: Some("u1".to_owned()),
        optional_params: params(&[
            ("temperature", number(0.2)),
            ("custom_llm_provider", text("azure")),
        ]),
        additional_params: params(&[
            (
                "stream_options",
                object(&[("include_usage", Json::Bool(true))]),
            ),
            ("custom_llm_provider", text("openai")),
        ]),
    }
}

fn empty_update() -> CallUpdate {
    CallUpdate {
        model: None,
        user: None,
        optional_params: params(&[]),
        additional_params: params(&[]),
    }
}

fn user_cleared_outcome() -> String {
    let mut state = build(init());
    let mut filled = empty_update();
    filled.user = Some("u1".to_owned());
    update_environment_variables(&mut state, &params(&[]), filled)
        .expect("the fixture update succeeds");
    update_environment_variables(&mut state, &params(&[]), empty_update())
        .expect("the fixture update succeeds");
    summary(&state, &["user"])
}

fn provider_wrong_type_outcome() -> String {
    let mut state = build(init());
    let mut update = empty_update();
    update.additional_params = params(&[(
        "custom_llm_provider",
        Json::Number(serde_json::Number::from(7_i32)),
    )]);
    update_environment_variables(&mut state, &params(&[]), update)
        .expect_err("a numeric provider is refused")
        .to_string()
}

fn provider_null_outcome() -> String {
    let mut state = build(init());
    let mut update = empty_update();
    update.additional_params = params(&[("custom_llm_provider", Json::Null)]);
    update_environment_variables(&mut state, &params(&[]), update)
        .expect("a null provider clears the field");
    encoded(&state.custom_llm_provider)
}

fn from_kwargs_outcome() -> String {
    let mut state = build(init());
    let kwargs = params(&[
        ("metadata", object(&[("a", number(1.0))])),
        (
            "litellm_metadata",
            object(&[("a", number(2.0)), ("b", number(3.0))]),
        ),
    ]);
    let caller = params(&[
        (
            "metadata",
            object(&[("a", number(9.0)), ("c", number(4.0))]),
        ),
        ("litellm_call_id", text("call-2")),
    ]);
    let update = CallUpdate {
        model: Some("gpt-4.1".to_owned()),
        user: None,
        optional_params: params(&[]),
        additional_params: params(&[("custom_llm_provider", text("openai"))]),
    };
    update_from_kwargs(&mut state, &kwargs, Some(&caller), update)
        .expect("the fixture update succeeds");
    format!(
        "{}/{}",
        encoded(&state.litellm_params),
        summary(
            &state,
            &[
                "litellm_params",
                "model",
                "litellm_call_id",
                "custom_llm_provider",
            ],
        )
    )
}

fn from_kwargs_metadata_only_outcome() -> String {
    let mut state = build(init());
    let kwargs = params(&[("litellm_metadata", object(&[("b", number(3.0))]))]);
    update_from_kwargs(&mut state, &kwargs, None, empty_update())
        .expect("the fixture update succeeds");
    encoded(&state.litellm_params)
}

fn from_kwargs_empty_caller_metadata_outcome() -> String {
    let mut state = build(init());
    let kwargs = params(&[("metadata", object(&[("a", number(1.0))]))]);
    let caller = params(&[("metadata", object(&[]))]);
    update_from_kwargs(&mut state, &kwargs, Some(&caller), empty_update())
        .expect("the fixture update succeeds");
    encoded(&state.litellm_params)
}

fn from_kwargs_non_object_metadata_outcome() -> String {
    let mut state = build(init());
    let kwargs = params(&[("metadata", text("ignored"))]);
    update_from_kwargs(&mut state, &kwargs, None, empty_update())
        .expect("the fixture update succeeds");
    encoded(&state.litellm_params)
}

fn messages_updated_outcome() -> String {
    let mut state = build(init());
    update_messages(
        &mut state,
        vec![object(&[("role", text("user")), ("content", text("hi"))])],
    );
    format!(
        "{}/{}",
        encoded(&state.messages),
        encoded(&state.model_call_details.get("messages").cloned())
    )
}

fn outcome(scenario: &Scenario) -> String {
    match scenario {
        Scenario::Environment => environment(
            call_update(),
            &params(&[
                ("api_base", text("http://proxy")),
                ("litellm_request_debug", Json::Bool(true)),
                (
                    "stream_options",
                    object(&[("include_usage", Json::Bool(true))]),
                ),
                ("input_cost_per_token", number(0.5)),
            ]),
        ),
        Scenario::ModelKept => environment(empty_update(), &params(&[])),
        Scenario::UserCleared => user_cleared_outcome(),
        Scenario::PricingNull => environment(
            empty_update(),
            &params(&[("input_cost_per_token", Json::Null)]),
        ),
        Scenario::PricingAbsent => environment(
            empty_update(),
            &params(&[("api_base", text("http://proxy"))]),
        ),
        Scenario::ProviderWrongType => provider_wrong_type_outcome(),
        Scenario::ProviderNull => provider_null_outcome(),
        Scenario::FromKwargs => from_kwargs_outcome(),
        Scenario::FromKwargsMetadataOnly => from_kwargs_metadata_only_outcome(),
        Scenario::FromKwargsEmptyCallerMetadata => from_kwargs_empty_caller_metadata_outcome(),
        Scenario::FromKwargsNonObjectMetadata => from_kwargs_non_object_metadata_outcome(),
        Scenario::MessagesUpdated => messages_updated_outcome(),
    }
}

#[rstest]
#[case::environment_update_merges_and_overrides(Scenario::Environment, r#"gpt-4.1/"u1"/true/{"include_usage":true}/true/"openai"/{"api_base":"http://proxy","litellm_request_debug":true,"stream_options":{"include_usage":true},"input_cost_per_token":0.5}/{"model":"gpt-4.1","messages":[{"role":"user","content":"hi"}],"optional_params":{"temperature":0.2,"custom_llm_provider":"azure"},"litellm_params":{"api_base":"http://proxy","litellm_request_debug":true,"stream_options":{"include_usage":true},"input_cost_per_token":0.5},"start_time":"2026-01-01T00:00:00+00:00","stream":false,"user":"u1","call_type":"completion","custom_llm_provider":"openai","stream_options":{"include_usage":true},"standard_callback_dynamic_params":{"langfuse_host":"h"}}"#)]
#[case::a_missing_model_keeps_the_previous_one(Scenario::ModelKept, r#"gpt-4o/null/false/null/false/null/{}/{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"optional_params":{},"litellm_params":{},"start_time":"2026-01-01T00:00:00+00:00","stream":false,"user":null,"call_type":"completion","custom_llm_provider":"<missing>","stream_options":"<missing>","standard_callback_dynamic_params":{"langfuse_host":"h"}}"#)]
#[case::a_missing_user_clears_the_field(Scenario::UserCleared, r#"{"user":null}"#)]
#[case::a_null_pricing_key_is_not_custom_pricing(Scenario::PricingNull, r#"gpt-4o/null/false/null/false/null/{"input_cost_per_token":null}/{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"optional_params":{},"litellm_params":{"input_cost_per_token":null},"start_time":"2026-01-01T00:00:00+00:00","stream":false,"user":null,"call_type":"completion","custom_llm_provider":"<missing>","stream_options":"<missing>","standard_callback_dynamic_params":{"langfuse_host":"h"}}"#)]
#[case::a_missing_pricing_key_is_not_custom_pricing(Scenario::PricingAbsent, r#"gpt-4o/null/false/null/false/null/{"api_base":"http://proxy"}/{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"optional_params":{},"litellm_params":{"api_base":"http://proxy"},"start_time":"2026-01-01T00:00:00+00:00","stream":false,"user":null,"call_type":"completion","custom_llm_provider":"<missing>","stream_options":"<missing>","standard_callback_dynamic_params":{"langfuse_host":"h"}}"#)]
#[case::a_wrongly_typed_provider_names_the_field(
    Scenario::ProviderWrongType,
    "the custom_llm_provider value must be a string"
)]
#[case::a_null_provider_clears_the_field(Scenario::ProviderNull, "null")]
#[case::kwargs_metadata_loses_to_caller_litellm_params(Scenario::FromKwargs, r#"{"metadata":{"a":1.0,"c":4.0},"litellm_metadata":{"a":2.0,"b":3.0},"litellm_call_id":"call-2"}/{"litellm_params":{"metadata":{"a":1.0,"c":4.0},"litellm_metadata":{"a":2.0,"b":3.0},"litellm_call_id":"call-2"},"model":"gpt-4.1","litellm_call_id":"call-1","custom_llm_provider":"openai"}"#)]
#[case::litellm_metadata_fills_an_absent_metadata(
    Scenario::FromKwargsMetadataOnly,
    r#"{"litellm_metadata":{"b":3.0},"metadata":{"b":3.0}}"#
)]
#[case::empty_caller_metadata_adds_no_key(
    Scenario::FromKwargsEmptyCallerMetadata,
    r#"{"metadata":{"a":1.0}}"#
)]
#[case::non_object_kwargs_metadata_is_ignored(Scenario::FromKwargsNonObjectMetadata, "{}")]
#[case::update_messages_touches_both_places(
    Scenario::MessagesUpdated,
    r#"[{"role":"user","content":"hi"}]/[{"role":"user","content":"hi"}]"#
)]
fn logging_state_behavior_tests_behavior(#[case] scenario: Scenario, #[case] expected: &str) {
    assert_eq!(outcome(&scenario), expected);
}
