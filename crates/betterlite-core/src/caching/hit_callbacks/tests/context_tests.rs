use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{base_hit, cached, details, encoded, logging_state, object, params, text};
use crate::caching::hit_callbacks::context::{
    CacheHitContext, CacheHitError, record_cache_hit_context,
};

fn recorded<Adjust>(
    kwargs: &IndexMap<String, Json>,
    cached_result: &Json,
    adjust: Adjust,
) -> LoggingState
where
    Adjust: FnOnce(&mut CacheHitContext<'_>),
{
    let mut state = logging_state(None);
    let mut hit = base_hit(kwargs, cached_result);
    adjust(&mut hit);
    record_cache_hit_context(&mut state, &hit).expect("the cache hit is recorded");
    state
}

fn full_kwargs() -> IndexMap<String, Json> {
    params(&[
        (
            "messages",
            Json::List(vec![object(&[
                ("role", text("user")),
                ("content", text("hi")),
            ])]),
        ),
        ("api_base", text("https://api.openai.com")),
        ("metadata", object(&[("key", text("value"))])),
        ("model_info", object(&[("id", text("m1"))])),
        ("proxy_server_request", object(&[("method", text("POST"))])),
        ("stream_response", object(&[("chunk", text("c"))])),
        ("logger_fn", text("hook")),
        ("user", text("u1")),
        ("stream", Json::Bool(true)),
        ("custom_llm_provider", text("ignored-from-kwargs")),
        (
            "litellm_params",
            object(&[("preset_cache_key", text("derived-key"))]),
        ),
    ])
}

#[test]
fn records_the_request_environment_on_a_cache_hit() {
    let kwargs = full_kwargs();
    let cached_result = cached();
    let state = recorded(&kwargs, &cached_result, |_hit| {});
    assert_eq!(state.model, "gpt-4o");
    assert_eq!(state.user.as_deref(), Some("u1"));
    assert_eq!(state.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        encoded(&Json::Object(state.litellm_params)),
        r#"{"logger_fn":"hook","acompletion":true,"api_base":"https://api.openai.com","metadata":{"key":"value"},"model_info":{"id":"m1"},"proxy_server_request":{"method":"POST"},"stream_response":{"chunk":"c"},"custom_llm_provider":"openai","preset_cache_key":"preset-key"}"#
    );
}

#[test]
fn projects_the_cached_result_into_the_call_details() {
    let kwargs = full_kwargs();
    let cached_result = cached();
    let state = recorded(&kwargs, &cached_result, |_hit| {});
    assert_eq!(
        details(
            &state,
            &[
                "input",
                "stream",
                "custom_llm_provider",
                "original_response",
                "api_key",
                "additional_args",
                "caching_details",
            ],
        ),
        r#"{"input":[{"role":"user","content":"hi"}],"stream":true,"custom_llm_provider":"openai","original_response":{"id":"chatcmpl-1","object":"chat.completion","choices":[]},"api_key":null,"additional_args":null,"caching_details":{"cache_hit":true,"cache_duration_ms":12.5}}"#
    );
}

#[test]
fn records_the_defaults_when_the_request_omits_the_optional_values() {
    let kwargs = params(&[]);
    let cached_result = cached();
    let state = recorded(&kwargs, &cached_result, |hit| {
        hit.custom_llm_provider = None;
        hit.preset_cache_key = None;
        hit.cache_duration_ms = None;
    });
    assert_eq!(state.user, None);
    assert_eq!(state.custom_llm_provider, None);
    assert_eq!(
        encoded(&Json::Object(state.litellm_params.clone())),
        r#"{"logger_fn":null,"acompletion":true,"api_base":"","metadata":{},"model_info":{},"proxy_server_request":null,"stream_response":{},"custom_llm_provider":null,"preset_cache_key":null}"#
    );
    assert_eq!(
        details(&state, &["input", "stream", "caching_details", "api_key"],),
        r#"{"input":"","stream":false,"caching_details":{"cache_hit":true,"cache_duration_ms":null},"api_key":null}"#
    );
}

#[test]
fn a_present_null_kwarg_stays_null() {
    let kwargs = params(&[
        ("api_base", Json::Null),
        ("metadata", Json::Null),
        ("model_info", Json::Null),
        ("stream_response", Json::Null),
        ("messages", Json::Null),
        ("stream", Json::Null),
        ("user", Json::Null),
    ]);
    let cached_result = cached();
    let state = recorded(&kwargs, &cached_result, |_hit| {});
    assert_eq!(state.user, None);
    assert_eq!(
        details(&state, &["input", "stream"]),
        r#"{"input":null,"stream":null}"#
    );
    assert_eq!(
        encoded(&Json::Object(state.litellm_params)),
        r#"{"logger_fn":null,"acompletion":true,"api_base":null,"metadata":null,"model_info":null,"proxy_server_request":null,"stream_response":null,"custom_llm_provider":"openai","preset_cache_key":"preset-key"}"#
    );
}

#[test]
fn a_sync_cache_hit_records_a_sync_call() {
    let kwargs = params(&[]);
    let cached_result = cached();
    let state = recorded(&kwargs, &cached_result, |hit| hit.is_async = false);
    assert_eq!(
        state.litellm_params.get("acompletion"),
        Some(&Json::Bool(false))
    );
}

#[test]
fn an_embedding_cache_hit_projects_the_input_field() {
    let kwargs = params(&[
        ("messages", text("ignored")),
        ("input", Json::List(vec![text("one"), text("two")])),
    ]);
    let cached_result = cached();
    let state = recorded(&kwargs, &cached_result, |hit| hit.is_embedding = true);
    assert_eq!(details(&state, &["input"]), r#"{"input":["one","two"]}"#);
}

#[test]
fn a_missing_handler_preset_key_reads_the_kwarg_key() {
    let kwargs = params(&[(
        "litellm_params",
        object(&[("preset_cache_key", text("derived-key"))]),
    )]);
    let cached_result = cached();
    let state = recorded(&kwargs, &cached_result, |hit| {
        hit.preset_cache_key = None;
    });
    assert_eq!(
        state.litellm_params.get("preset_cache_key"),
        Some(&text("derived-key"))
    );
}

#[test]
fn an_empty_handler_preset_key_is_used_as_given() {
    let kwargs = params(&[(
        "litellm_params",
        object(&[("preset_cache_key", text("derived-key"))]),
    )]);
    let cached_result = cached();
    let state = recorded(&kwargs, &cached_result, |hit| {
        hit.preset_cache_key = Some("");
    });
    assert_eq!(
        state.litellm_params.get("preset_cache_key"),
        Some(&text(""))
    );
}

#[test]
fn the_caller_kwargs_are_not_changed() {
    let kwargs = params(&[
        ("api_base", text("https://api.openai.com")),
        ("user", text("u1")),
        ("stream", Json::Bool(true)),
    ]);
    let before = kwargs.clone();
    let cached_result = cached();
    let _state = recorded(&kwargs, &cached_result, |_hit| {});
    assert_eq!(kwargs, before);
}

#[test]
fn the_preset_key_is_copied_into_the_logging_state() {
    let mut state = logging_state(None);
    let recorded_key = {
        let kwargs = params(&[(
            "litellm_params",
            object(&[("preset_cache_key", text("derived-key"))]),
        )]);
        let cached_result = cached();
        let mut hit = base_hit(&kwargs, &cached_result);
        hit.preset_cache_key = None;
        record_cache_hit_context(&mut state, &hit).expect("the cache hit is recorded");
        state.litellm_params.get("preset_cache_key").cloned()
    };
    assert_eq!(recorded_key, Some(text("derived-key")));
}

#[rstest]
#[case::api_base("api_base")]
#[case::user("user")]
#[case::metadata("metadata")]
#[case::model_info("model_info")]
#[case::proxy_server_request("proxy_server_request")]
#[case::stream_response("stream_response")]
fn a_wrongly_typed_kwarg_names_the_field(#[case] field: &'static str) {
    let kwargs = params(&[(field, Json::Bool(true))]);
    let cached_result = cached();
    let mut state = logging_state(None);
    let hit = base_hit(&kwargs, &cached_result);
    let error = record_cache_hit_context(&mut state, &hit).expect_err("the value is refused");
    assert_eq!(error, CacheHitError::FieldType { field });
    assert!(error.to_string().contains(field));
}

#[test]
fn a_wrongly_typed_stream_flag_names_the_field() {
    let kwargs = params(&[("stream", text("yes"))]);
    let cached_result = cached();
    let mut state = logging_state(None);
    let hit = base_hit(&kwargs, &cached_result);
    let error = record_cache_hit_context(&mut state, &hit).expect_err("the value is refused");
    assert_eq!(error, CacheHitError::FieldType { field: "stream" });
}

#[rstest]
#[case::litellm_params(
    params(&[("litellm_params", text("not-an-object"))]),
    "litellm_params"
)]
#[case::preset_cache_key(
    params(&[(
        "litellm_params",
        object(&[("preset_cache_key", Json::Bool(true))]),
    )]),
    "preset_cache_key"
)]
fn a_wrongly_typed_internal_value_names_the_field(
    #[case] kwargs: IndexMap<String, Json>,
    #[case] field: &'static str,
) {
    let cached_result = cached();
    let mut state = logging_state(None);
    let mut hit = base_hit(&kwargs, &cached_result);
    hit.preset_cache_key = None;
    let error = record_cache_hit_context(&mut state, &hit).expect_err("the value is refused");
    assert_eq!(error, CacheHitError::FieldType { field });
}
