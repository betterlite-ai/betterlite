#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_models::decisions::Json as ModelJson;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::SecretString;
use uuid::Uuid;

use super::chat::{
    BASE_OPENAI_PARAMS, ChatTransform, DeepInfraChatSettings, MapRequest, map_openai_params,
    supported_openai_params, transform_request,
};
use super::rerank::response::{
    BaseLlmException, LoggingFailure, RerankDecodeError, RerankLogging, ResponseRequest,
    decode_deepinfra_error, decode_deepinfra_rerank, get_error_class, transform_rerank_response,
};
use super::rerank::{
    EnvironmentRequest, Headers, MapRequest as RerankMapRequest, Params, RerankError, RerankLimits,
    SUPPORTED_RERANK_PARAMS, complete_url, map_cohere_rerank_params, transform_rerank_request,
    validate_environment,
};
use crate::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use crate::api::vector_store_records::records::score::Score;
use crate::chat::ChatMessage;
use crate::chat_content::{ChatContentError, Mode, RefuseDownloads};
use crate::reasoning_models::DropParams;

const MISTRAL: &str = "mistralai/Mistral-7B-Instruct-v0.1";
const LIMIT: usize = 65_536;

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn model_object(text: &str) -> ModelJson {
    serde_json::from_str(text).unwrap()
}

fn model_number(text: &str) -> ModelJson {
    serde_json::from_str(text).unwrap()
}

fn params(text: &str) -> Params {
    serde_json::from_str(text).unwrap()
}

fn limits() -> RerankLimits {
    RerankLimits {
        max_body_bytes: LIMIT,
    }
}

fn generated() -> Uuid {
    Uuid::from_u128(7)
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn messages(text: &str) -> Vec<ChatMessage> {
    serde_json::from_str(text).unwrap()
}

fn environment() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn chat_transform(
    model: &str,
    messages: &[ChatMessage],
    optional_params: &Object,
    environment: &BTreeMap<String, String>,
) -> Result<Object, ChatContentError> {
    transform_request(
        &ChatTransform {
            model,
            messages,
            optional_params,
            environment,
            downloads: &RefuseDownloads,
        },
        Mode::Sync,
    )
}

#[test]
fn the_supported_list_is_the_pinned_list_with_reasoning_last() {
    let base: Vec<&str> = BASE_OPENAI_PARAMS.split_whitespace().collect();
    assert_eq!(supported_openai_params(false), base);
    let mut expected = base.clone();
    expected.push("reasoning_effort");
    assert_eq!(supported_openai_params(true), expected);
}

#[test]
fn captured_settings_start_absent_and_keep_only_what_a_caller_supplied() {
    let default = DeepInfraChatSettings::default();
    assert_eq!(default.get_config(), Object::new());
    let supplied = DeepInfraChatSettings::with_settings(object(
        r#"{"temperature": 0.5, "tool_choice": "auto", "stop": null}"#,
    ))
    .unwrap();
    assert_eq!(
        supplied.get_config(),
        object(r#"{"temperature": 0.5, "tool_choice": "auto"}"#)
    );
}

#[test]
fn captured_settings_reject_a_name_the_constructor_does_not_take() {
    let error = DeepInfraChatSettings::with_settings(object(r#"{"thinking": "on"}"#)).unwrap_err();
    assert_eq!(error.status_code(), 500);
}

#[rstest]
#[case::auto(r#"{"tool_choice": "auto"}"#, false, false, None)]
#[case::none(r#"{"tool_choice": "none"}"#, false, false, None)]
#[case::named_dropped_by_request_flag(
    r#"{"tool_choice": {"type": "function", "function": {"name": "lookup"}}}"#,
    true,
    false,
    None
)]
#[case::named_dropped_by_global_flag(
    r#"{"tool_choice": {"type": "function", "function": {"name": "lookup"}}}"#,
    false,
    true,
    None
)]
#[case::named_kept_as_error(
    r#"{"tool_choice": {"type": "function", "function": {"name": "lookup"}}}"#,
    false,
    false,
    Some(400)
)]
#[case::required_kept_as_error(r#"{"tool_choice": "required"}"#, false, false, Some(400))]
fn the_mapper_never_inserts_tool_choice(
    #[case] non_default: &str,
    #[case] request_flag: bool,
    #[case] global_flag: bool,
    #[case] expected_status: Option<u16>,
) {
    let given = object(non_default);
    let mut optional = Object::new();
    let outcome = map_openai_params(MapRequest {
        model: "meta-llama/Meta-Llama-3-70B",
        non_default_params: &given,
        optional_params: &mut optional,
        drop_params: DropParams {
            global: global_flag,
            request: request_flag,
        },
        minimum_temperature: 0.0001,
        supports_reasoning: false,
    });
    match expected_status {
        Some(status) => {
            let error = outcome.unwrap_err();
            assert_eq!(error.status_code(), status);
        }
        None => {
            outcome.unwrap();
            assert_eq!(optional, Object::new());
        }
    }
}

#[rstest]
#[case::mistral_zero(MISTRAL, r#"{"temperature": 0}"#, true)]
#[case::mistral_zero_float(MISTRAL, r#"{"temperature": 0.0}"#, true)]
#[case::mistral_negative_zero(MISTRAL, r#"{"temperature": -0.0}"#, true)]
#[case::mistral_nonzero(MISTRAL, r#"{"temperature": 0.2}"#, false)]
#[case::other_model("meta-llama/Meta-Llama-3-70B", r#"{"temperature": 0}"#, false)]
fn only_the_mistral_model_replaces_a_zero_temperature(
    #[case] model: &str,
    #[case] non_default: &str,
    #[case] replaced: bool,
) {
    let given = object(non_default);
    let mut optional = Object::new();
    map_openai_params(MapRequest {
        model,
        non_default_params: &given,
        optional_params: &mut optional,
        drop_params: DropParams::default(),
        minimum_temperature: 0.25,
        supports_reasoning: false,
    })
    .unwrap();
    let expected = if replaced {
        object(r#"{"temperature": 0.25}"#)
    } else {
        given.clone()
    };
    assert_eq!(optional, expected);
}

#[rstest]
#[case::renamed_even_when_null(r#"{"max_completion_tokens": null}"#, r#"{"max_tokens": null}"#)]
#[case::renamed(r#"{"max_completion_tokens": 9}"#, r#"{"max_tokens": 9}"#)]
#[case::null_supported_is_skipped(r#"{"stop": null}"#, "{}")]
#[case::unsupported_is_ignored(r#"{"seed": 4}"#, "{}")]
#[case::supported_values_are_copied(
    r#"{"stop": ["END"], "functions": [], "logit_bias": {"1": 2}}"#,
    r#"{"stop": ["END"], "functions": [], "logit_bias": {"1": 2}}"#
)]
#[case::reasoning_effort_needs_the_capability(r#"{"reasoning_effort": "high"}"#, "{}")]
fn the_mapper_renames_the_token_limit_and_copies_supported_values(
    #[case] non_default: &str,
    #[case] expected: &str,
) {
    let given = object(non_default);
    let mut optional = Object::new();
    map_openai_params(MapRequest {
        model: "meta-llama/Meta-Llama-3-70B",
        non_default_params: &given,
        optional_params: &mut optional,
        drop_params: DropParams::default(),
        minimum_temperature: 0.0001,
        supports_reasoning: false,
    })
    .unwrap();
    assert_eq!(optional, object(expected));
}

#[test]
fn a_reasoning_capable_model_advertises_and_copies_reasoning_effort() {
    let given = object(r#"{"reasoning_effort": "high"}"#);
    let mut optional = Object::new();
    map_openai_params(MapRequest {
        model: "meta-llama/Meta-Llama-3-70B",
        non_default_params: &given,
        optional_params: &mut optional,
        drop_params: DropParams::default(),
        minimum_temperature: 0.0001,
        supports_reasoning: true,
    })
    .unwrap();
    assert_eq!(optional, object(r#"{"reasoning_effort": "high"}"#));
}

#[test]
fn a_later_input_entry_overwrites_an_earlier_one() {
    let given = object(r#"{"max_completion_tokens": 2, "max_tokens": 7}"#);
    let mut optional = Object::new();
    map_openai_params(MapRequest {
        model: "m",
        non_default_params: &given,
        optional_params: &mut optional,
        drop_params: DropParams::default(),
        minimum_temperature: 0.0001,
        supports_reasoning: false,
    })
    .unwrap();
    assert_eq!(optional, object(r#"{"max_tokens": 7}"#));
}

fn tool_messages(text: &str) -> Vec<ModelJson> {
    let body = chat_transform("m", &messages(text), &Object::new(), &environment()).unwrap();
    let encoded = serde_json::to_value(body.get("messages").unwrap()).unwrap();
    serde_json::from_value(encoded).unwrap()
}

#[test]
fn a_single_text_tool_part_becomes_its_text_value() {
    let transformed = tool_messages(
        r#"[{"role": "tool", "content": [{"type": "text", "text": "20"}], "tool_call_id": "c"}]"#,
    );
    assert_eq!(
        transformed,
        vec![model_object(
            r#"{"role": "tool", "content": "20", "tool_call_id": "c"}"#
        )]
    );
}

#[test]
fn a_single_text_tool_part_with_a_null_text_keeps_null() {
    let transformed = tool_messages(
        r#"[{"role": "tool", "content": [{"type": "text", "text": null}], "tool_call_id": "c"}]"#,
    );
    assert_eq!(
        transformed,
        vec![model_object(
            r#"{"role": "tool", "content": null, "tool_call_id": "c"}"#
        )]
    );
}

#[test]
fn any_other_tool_content_list_becomes_python_json_text() {
    let transformed = tool_messages(
        r#"[{"role": "tool", "content": [{"type": "text", "text": "café"}, {"type": "image_url", "image_url": {"url": "x"}}]}]"#,
    );
    let expected = serde_json::json!({
        "role": "tool",
        "content": r#"[{"type": "text", "text": "café"}, {"type": "image_url", "image_url": {"url": "x"}}]"#,
    });
    assert_eq!(serde_json::to_value(&transformed[0]).unwrap(), expected);
}

#[test]
fn another_role_keeps_its_content_list() {
    let transformed = tool_messages(
        r#"[{"role": "user", "content": [{"type": "text", "text": "hi"}]}, {"role": "tool", "content": "plain"}]"#,
    );
    assert_eq!(
        transformed,
        vec![
            model_object(r#"{"role": "user", "content": [{"type": "text", "text": "hi"}]}"#),
            model_object(r#"{"role": "tool", "content": "plain"}"#),
        ]
    );
}

#[test]
fn the_parent_conversion_runs_before_the_tool_cleanup() {
    let transformed = tool_messages(
        r#"[{"role": "tool", "content": [{"type": "tool_reference", "tool_name": "lookup"}, {"type": "text", "text": "done"}]}]"#,
    );
    assert_eq!(
        transformed,
        vec![model_object(r#"{"role": "tool", "content": "done"}"#)]
    );
}

#[rstest]
#[case::openai_removed(
    Some("https://api.deepinfra.com/v1/openai"),
    "out/openai/chat",
    "https://api.deepinfra.com/v1/inference/out/openai/chat"
)]
#[case::trailing_slashes("https://host/v1///", "m", "https://host/v1/inference/m")]
#[case::bare_host("https://host", "m", "https://host/inference/m")]
fn the_rerank_url_removes_every_openai_substring(
    #[case] api_base: Option<&str>,
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(complete_url(api_base, model).unwrap(), expected);
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn the_rerank_url_requires_a_truthy_base(#[case] api_base: Option<&str>) {
    assert_eq!(
        complete_url(api_base, "m"),
        Err(RerankError::MissingApiBase)
    );
}

#[test]
fn the_rerank_headers_merge_caller_values_over_the_defaults() {
    let caller = Headers::from([(String::from("X-Extra"), String::from("1"))]);
    let key = secret("sk-test");
    let headers = validate_environment(&EnvironmentRequest {
        headers: &caller,
        api_key: Some(&key),
        environment_key: None,
    })
    .unwrap();
    assert_eq!(
        headers,
        Headers::from([
            (
                String::from("Authorization"),
                String::from("Bearer sk-test")
            ),
            (String::from("accept"), String::from("application/json")),
            (
                String::from("content-type"),
                String::from("application/json")
            ),
            (String::from("X-Extra"), String::from("1")),
        ])
    );
}

#[test]
fn a_caller_authorization_header_wins_over_the_bearer_default() {
    let header = Headers::from([(String::from("Authorization"), String::from("Token other"))]);
    let key = secret("sk-test");
    let merged = validate_environment(&EnvironmentRequest {
        headers: &header,
        api_key: Some(&key),
        environment_key: None,
    })
    .unwrap();
    assert_eq!(
        merged.get("Authorization").map(String::as_str),
        Some("Token other")
    );
}

#[rstest]
#[case::missing(None, None, true)]
#[case::environment_used(None, Some("env-key"), false)]
#[case::empty_explicit_is_kept(Some(""), Some("env-key"), false)]
fn the_rerank_key_falls_back_only_for_a_missing_key(
    #[case] explicit_key: Option<&str>,
    #[case] environment_key: Option<&str>,
    #[case] fails: bool,
) {
    let supplied = explicit_key.map(secret);
    let fallback = environment_key.map(secret);
    let headers = Headers::new();
    let merged = validate_environment(&EnvironmentRequest {
        headers: &headers,
        api_key: supplied.as_ref(),
        environment_key: fallback.as_ref(),
    });
    assert_eq!(merged.is_err(), fails);
    if let Ok(values) = merged {
        let expected = format!(
            "Bearer {}",
            explicit_key.or(environment_key).unwrap_or_default()
        );
        assert_eq!(values.get("Authorization"), Some(&expected));
    }
}

#[test]
fn the_rerank_supported_list_is_query_and_documents() {
    assert_eq!(SUPPORTED_RERANK_PARAMS, ["query", "documents"]);
}

#[test]
fn a_truthy_query_repeats_for_every_formal_document() {
    let documents = [
        RerankDocument::Text(String::from("a")),
        RerankDocument::Text(String::from("b")),
    ];
    let optional = map_cohere_rerank_params(&RerankMapRequest {
        non_default_params: &Params::new(),
        query: "find",
        documents: &documents,
    });
    assert_eq!(optional, params(r#"{"queries": ["find", "find"]}"#));
}

#[test]
fn documents_come_from_the_parameters_and_other_entries_are_ignored() {
    let optional = map_cohere_rerank_params(&RerankMapRequest {
        non_default_params: &params(
            r#"{"documents": ["a"], "service_tier": "scale", "instruction": "rank", "webhook": "w", "queries": ["q"], "other": 1}"#,
        ),
        query: "",
        documents: &[],
    });
    assert_eq!(
        optional,
        params(
            r#"{"queries": ["q"], "documents": ["a"], "service_tier": "scale", "instruction": "rank", "webhook": "w"}"#
        )
    );
}

#[test]
fn a_null_overlay_entry_is_ignored() {
    let optional = map_cohere_rerank_params(&RerankMapRequest {
        non_default_params: &params(r#"{"queries": null, "documents": null, "instruction": null}"#),
        query: "find",
        documents: &[RerankDocument::Text(String::from("a"))],
    });
    assert_eq!(optional, params(r#"{"queries": ["find"]}"#));
}

#[test]
fn the_request_transform_copies_or_starts_empty() {
    let given = params(r#"{"queries": ["q"]}"#);
    assert_eq!(transform_rerank_request(Some(&given)), given);
    assert_eq!(transform_rerank_request(None), Params::new());
}

struct Recording {
    calls: Vec<String>,
    fail: bool,
}

impl Recording {
    fn new(fail: bool) -> Self {
        Self {
            calls: Vec::new(),
            fail,
        }
    }
}

impl RerankLogging for Recording {
    fn post_call(&mut self, original_response: &str) -> Result<(), LoggingFailure> {
        if self.fail {
            return Err(LoggingFailure);
        }
        self.calls.push(original_response.to_owned());
        Ok(())
    }
}

fn project(body: &[u8], logging: &mut Recording, model_response: RerankResponse) -> RerankResponse {
    transform_rerank_response(
        model_response,
        ResponseRequest {
            model: "rerank-model",
            body,
            logging,
            generated: generated(),
            limits: limits(),
        },
    )
}

fn supplied_response() -> RerankResponse {
    RerankResponse {
        id: Some(String::from("supplied")),
        results: Some(Vec::new()),
        meta: None,
        hidden_params: BTreeMap::from([(String::from("kept"), ModelJson::Bool(true))]),
    }
}

#[test]
fn scores_keep_their_order_and_become_enumerated_results() {
    let body = br#"{"scores": [0.9, 0.125], "request_id": "r"}"#;
    let projected = project(body, &mut Recording::new(false), RerankResponse::default());
    let results = projected.results.unwrap();
    assert_eq!(
        results
            .iter()
            .map(|result| result.index)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(
        results
            .iter()
            .map(|result| result.relevance_score.clone())
            .collect::<Vec<_>>(),
        vec![
            Score::try_from(0.9_f64).unwrap(),
            Score::try_from(0.125_f64).unwrap()
        ]
    );
    assert_eq!(projected.id.as_deref(), Some("r"));
}

#[test]
fn absent_scores_yield_an_empty_result_and_zero_tokens() {
    let projected = project(b"{}", &mut Recording::new(false), RerankResponse::default());
    assert_eq!(projected.results, Some(Vec::new()));
    let meta = projected.meta.unwrap();
    assert_eq!(meta.tokens.unwrap().unwrap().input_tokens, Some(Some(0)));
    assert_eq!(
        meta.billed_units.unwrap().unwrap().total_tokens,
        Some(Some(0))
    );
    assert_eq!(projected.id.as_deref(), Some(&generated().to_string()));
}

#[test]
fn null_input_tokens_stays_null_and_counts_no_billed_tokens() {
    let projected = project(
        br#"{"input_tokens": null}"#,
        &mut Recording::new(false),
        RerankResponse::default(),
    );
    let meta = projected.meta.unwrap();
    assert_eq!(meta.tokens.unwrap().unwrap().input_tokens, Some(None));
    assert_eq!(meta.billed_units.unwrap().unwrap().total_tokens, Some(None));
}

#[test]
fn the_inference_status_fills_the_hidden_parameters() {
    let body = br#"{"inference_status": {"status": "ok", "runtime_ms": 12, "cost": 0.5, "tokens_generated": 3, "tokens_input": 4}}"#;
    let projected = project(body, &mut Recording::new(false), RerankResponse::default());
    assert_eq!(
        projected.hidden_params,
        BTreeMap::from([
            (String::from("status"), ModelJson::Text(String::from("ok"))),
            (String::from("runtime_ms"), model_number("12")),
            (String::from("cost"), model_number("0.5")),
            (String::from("tokens_generated"), model_number("3")),
            (String::from("tokens_input"), model_number("4")),
            (
                String::from("model"),
                ModelJson::Text(String::from("rerank-model"))
            ),
        ])
    );
}

#[test]
fn the_inference_defaults_match_the_pinned_literals() {
    let projected = project(
        b"{\"scores\": [0.5]}",
        &mut Recording::new(false),
        RerankResponse::default(),
    );
    assert_eq!(
        projected.hidden_params.get("status"),
        Some(&ModelJson::Text(String::from("unknown")))
    );
    assert_eq!(
        projected.hidden_params.get("runtime_ms"),
        Some(&model_number("0"))
    );
    assert_eq!(
        projected.hidden_params.get("cost"),
        Some(&model_number("0.0"))
    );
    assert_eq!(
        projected.hidden_params.get("tokens_generated"),
        Some(&model_number("0"))
    );
    assert_eq!(
        projected.hidden_params.get("tokens_input"),
        Some(&model_number("0"))
    );
}

#[test]
fn the_post_call_hook_sees_the_raw_response_text() {
    let body = b"{\"scores\": []}";
    let mut logging = Recording::new(false);
    project(body, &mut logging, RerankResponse::default());
    assert_eq!(
        logging.calls,
        vec![String::from_utf8_lossy(body).into_owned()]
    );
}

#[rstest]
#[case::malformed(b"not json".as_slice())]
#[case::not_an_object(b"[]".as_slice())]
#[case::non_utf8(b"\xff".as_slice())]
#[case::null_status(b"{\"inference_status\": null}".as_slice())]
#[case::scores_not_a_list(b"{\"scores\": 4}".as_slice())]
#[case::score_not_a_number(b"{\"scores\": [\"x\"]}".as_slice())]
#[case::request_id_not_a_string(b"{\"request_id\": 4}".as_slice())]
fn a_failed_projection_returns_the_supplied_response(#[case] body: &[u8]) {
    let projected = project(body, &mut Recording::new(false), supplied_response());
    assert_eq!(projected.id.as_deref(), Some("supplied"));
    assert_eq!(projected.results, Some(Vec::new()));
    assert_eq!(
        projected.hidden_params.get("kept"),
        Some(&ModelJson::Bool(true))
    );
    assert_eq!(
        projected.hidden_params.get("model"),
        Some(&ModelJson::Text(String::from("rerank-model")))
    );
}

#[test]
fn an_oversized_body_is_refused_before_the_hook_runs() {
    let mut logging = Recording::new(false);
    let projected = transform_rerank_response(
        supplied_response(),
        ResponseRequest {
            model: "rerank-model",
            body: b"{\"scores\": []}",
            logging: &mut logging,
            generated: generated(),
            limits: RerankLimits { max_body_bytes: 4 },
        },
    );
    assert_eq!(projected.id.as_deref(), Some("supplied"));
    assert!(logging.calls.is_empty());
}

#[test]
fn a_failing_post_call_hook_falls_back_without_a_warning() {
    let projected = project(
        b"{\"scores\": [0.5]}",
        &mut Recording::new(true),
        supplied_response(),
    );
    assert_eq!(projected.id.as_deref(), Some("supplied"));
    assert_eq!(
        projected.hidden_params.get("model"),
        Some(&ModelJson::Text(String::from("rerank-model")))
    );
}

#[test]
fn the_byte_decoder_is_bounded_and_returns_json() {
    assert!(decode_deepinfra_rerank(b"{\"scores\": []}", limits()).is_ok());
    assert!(
        decode_deepinfra_rerank(b"{\"scores\": []}", RerankLimits { max_body_bytes: 14 }).is_ok()
    );
    assert!(matches!(
        decode_deepinfra_rerank(b"{\"scores\": []}", RerankLimits { max_body_bytes: 2 }),
        Err(RerankDecodeError::BodyTooLarge)
    ));
    assert!(matches!(
        decode_deepinfra_rerank(b"\xff", limits()),
        Err(RerankDecodeError::NotUtf8)
    ));
}

#[rstest]
#[case::nested_error(r#"{"detail": {"error": "boom"}}"#, "boom")]
#[case::string_detail(r#"{"detail": "slow down"}"#, "slow down")]
#[case::no_detail(r#"{"message": "kept"}"#, r#"{"message": "kept"}"#)]
#[case::detail_without_error(
    r#"{"detail": {"message": "kept"}}"#,
    r#"{"detail": {"message": "kept"}}"#
)]
#[case::malformed("{oops", "{oops")]
#[case::not_an_object("[1]", "[1]")]
#[case::non_string_error(r#"{"detail": {"error": 4}}"#, r#"{"detail": {"error": 4}}"#)]
fn error_extraction_prefers_the_nested_detail(#[case] message: &str, #[case] expected: &str) {
    assert_eq!(decode_deepinfra_error(message, limits()), expected);
}

#[test]
fn error_extraction_swallows_a_message_past_the_byte_limit() {
    let message = r#"{"detail": {"error": "boom"}}"#;
    assert_eq!(
        decode_deepinfra_error(message, RerankLimits { max_body_bytes: 2 }),
        message
    );
    assert_eq!(
        decode_deepinfra_error(
            message,
            RerankLimits {
                max_body_bytes: message.len()
            }
        ),
        "boom"
    );
}

#[test]
fn replays_the_deepinfra_payload_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/deepinfra_payload");
    let mut replayed = 0;
    for entry in fs::read_dir(corpus).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        let Some((&tag, payload)) = bytes.split_first() else {
            continue;
        };
        if tag % 2 == 0 {
            let _decoded = decode_deepinfra_rerank(payload, limits());
        } else if let Ok(message) = std::str::from_utf8(payload) {
            let _extracted = decode_deepinfra_error(message, limits());
        }
        replayed += 1;
    }
    assert!(replayed >= 2);
}

#[test]
fn the_error_class_keeps_the_status_and_headers_and_extracts_the_detail() {
    let headers = BTreeMap::from([(String::from("retry-after"), model_number("2"))]);
    let raised: BaseLlmException = get_error_class(
        r#"{"detail": {"error": "boom"}}"#,
        422,
        headers.clone(),
        limits(),
    );
    assert_eq!(raised.status_code, 422);
    assert_eq!(raised.message, "boom");
    assert_eq!(raised.headers, headers);
}
