#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::ready;
use std::path::PathBuf;
use std::sync::Arc;

use betterlite_config::sections::studio_identity::DEFAULT_API_BASE;
use betterlite_config::sections::studio_identity::Settings as StudioSettings;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use crate::credential_sources::getters::VertexParams;
use crate::endpoint_target::headers::set_headers;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::google_native_content::auth::{AuthTarget, NativeAuth, auth_token_and_url};
use crate::google_native_content::capabilities::{
    NativeProvider, SUPPORTED_OPTIONS, X_GOOGLE_API_KEY,
};
use crate::google_native_content::error::NativeContentError;
use crate::google_native_content::map::{
    camel_to_snake, normalize_response_schema, snake_to_camel, studio_optional_params,
    vertex_optional_params,
};
use crate::google_native_content::request::{
    GenerateContentBody, GenerateContentRequest, transform_request,
};
use crate::google_native_content::response::{
    GenerateContentResponse, RawResponse, decode_google_native_content, transform_response,
};
use crate::studio_identity::{AuthValue, NativeParams};
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

const MODEL: &str = "gemini-2.5-flash";
const OLD_MODEL: &str = "gemini-1.5-pro";
const CONTENTS: &str = r#"[{"role":"user","parts":[{"text":"hi"}]}]"#;

fn json_of(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn rendered<T>(value: &T) -> Json
where
    T: Serialize,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

fn settings() -> Settings {
    Settings::default()
}

fn request(provider: NativeProvider, model: &str, config: &str) -> GenerateContentBody {
    let request = GenerateContentRequest {
        model: model.to_owned(),
        contents: json_of(CONTENTS),
        tools: Json::Null,
        generation_config: object(config),
        system_instruction: Json::Null,
    };
    transform_request(provider, &request, &settings()).unwrap()
}

fn response_of(source: &str) -> GenerateContentResponse {
    decode_google_native_content(source.as_bytes()).unwrap()
}

fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn access_token(token: &str, project: &str) -> AccessToken {
    AccessToken {
        token: secret(token),
        project: project.to_owned(),
    }
}

fn auth<'text>(
    provider: NativeProvider,
    params: VertexParams<'text>,
    vertex: &'text VertexSettings,
    studio: &'text StudioSettings,
    keys: NativeParams,
) -> NativeAuth<'text> {
    NativeAuth {
        provider,
        model: MODEL,
        api_base: None,
        stream: false,
        params,
        vertex,
        studio,
        studio_keys: keys,
    }
}

fn header<'target>(target: &'target AuthTarget, name: &str) -> Option<&'target str> {
    target.headers.get(name).map(SecretString::expose_secret)
}

const SUPPORTED_SOURCE: [&str; 30] = [
    "http_options",
    "system_instruction",
    "temperature",
    "top_p",
    "top_k",
    "candidate_count",
    "max_output_tokens",
    "stop_sequences",
    "response_logprobs",
    "logprobs",
    "presence_penalty",
    "frequency_penalty",
    "seed",
    "response_mime_type",
    "response_schema",
    "response_json_schema",
    "routing_config",
    "model_selection_config",
    "safety_settings",
    "tools",
    "tool_config",
    "labels",
    "cached_content",
    "response_modalities",
    "media_resolution",
    "speech_config",
    "audio_timestamp",
    "automatic_function_calling",
    "thinking_config",
    "image_config",
];

#[test]
fn supported_options_match_the_source_list() {
    assert_eq!(SUPPORTED_OPTIONS, SUPPORTED_SOURCE);
}

#[test]
fn provider_names_match_litellm() {
    assert_eq!(NativeProvider::Studio.custom_llm_provider(), "gemini");
    assert_eq!(NativeProvider::Vertex.custom_llm_provider(), "vertex_ai");
}

#[rstest]
#[case::plain_snake("top_p", "topP")]
#[case::camel_stays("topP", "topP")]
#[case::multi_word("stop_sequences", "stopSequences")]
#[case::option_prefix("http_options", "httpOptions")]
#[case::instruction("system_instruction", "systemInstruction")]
#[case::schema("response_schema", "responseSchema")]
#[case::schema_camel("responseSchema", "responseSchema")]
#[case::one_word("labels", "labels")]
fn studio_map_keeps_supported_spellings_in_camel_case(#[case] given: &str, #[case] want: &str) {
    let source = format!(r#"{{"{given}":1}}"#);
    let expected = format!(r#"{{"{want}":1}}"#);
    assert_eq!(
        rendered(&studio_optional_params(&object(&source))),
        json_of(&expected)
    );
}

#[rstest]
#[case::snake_unknown("unknown_option")]
#[case::camel_unknown("unknownOption")]
#[case::acronym("HTTPOptions")]
#[case::not_an_option("fooBar")]
fn studio_map_drops_keys_the_capability_list_does_not_name(#[case] given: &str) {
    let source = format!(r#"{{"{given}":1}}"#);
    assert_eq!(
        rendered(&studio_optional_params(&object(&source))),
        json_of("{}")
    );
}

#[test]
fn studio_map_lets_the_later_spelling_win() {
    assert_eq!(
        rendered(&studio_optional_params(&object(r#"{"top_p":1,"topP":2}"#))),
        json_of(r#"{"topP":2}"#)
    );
}

#[rstest]
#[case::kept(r#"{"temperature":0.2}"#, r#"{"temperature":0.2}"#)]
#[case::unknown_to_snake(r#"{"unknownOption":"keep"}"#, r#"{"unknown_option":"keep"}"#)]
#[case::acronym_split(
    r#"{"HTTPOptions":{"timeout":1}}"#,
    r#"{"h_t_t_p_options":{"timeout":1}}"#
)]
#[case::candidate_count(r#"{"candidateCount":2}"#, r#"{"candidate_count":2}"#)]
#[case::json_schema(r#"{"responseJsonSchema":{}}"#, r#"{"response_json_schema":{}}"#)]
#[case::snake_kept(r#"{"custom_key":7}"#, r#"{"custom_key":7}"#)]
fn vertex_map_converts_every_key_without_filtering(#[case] given: &str, #[case] want: &str) {
    assert_eq!(
        rendered(&vertex_optional_params(&object(given))),
        json_of(want)
    );
}

#[test]
fn camel_and_snake_helpers_follow_the_source_spelling() {
    assert_eq!(camel_to_snake("HTTPOptions"), "h_t_t_p_options");
    assert_eq!(camel_to_snake("responseJsonSchema"), "response_json_schema");
    assert_eq!(snake_to_camel("response_json_schema"), "responseJsonSchema");
    assert_eq!(snake_to_camel("top_p"), "topP");
    assert_eq!(snake_to_camel("RESPONSE_SCHEMA"), "RESPONSESchema");
}

#[rstest]
#[case::camel_to_camel(
    "gemini-2.0-flash",
    r#"{"responseSchema":{"type":"object"}}"#,
    r#"{"responseJsonSchema":{"type":"object"}}"#
)]
#[case::snake_to_snake(
    "gemini-2.0-flash",
    r#"{"response_schema":{"type":"object"}}"#,
    r#"{"response_json_schema":{"type":"object"}}"#
)]
#[case::alias_wins(
    "gemini-2.0-flash",
    r#"{"responseSchema":{"type":"object"},"responseJsonSchema":{"type":"string"}}"#,
    r#"{"responseJsonSchema":{"type":"string"}}"#
)]
#[case::old_model("gemini-1.5-pro", r#"{"responseSchema":{"type":"object","properties":{"b":{"type":"string"},"a":{"type":"integer"}}},"responseJsonSchema":{"type":"object"}}"#, r#"{"responseSchema":{"type":"object","properties":{"b":{"type":"string"},"a":{"type":"integer"}},"propertyOrdering":["b","a"]}}"#)]
fn response_schema_normalization_matches_the_model(
    #[case] model: &str,
    #[case] given: &str,
    #[case] want: &str,
) {
    let mut config = object(given);
    normalize_response_schema(&mut config, model, &settings()).unwrap();
    assert_eq!(rendered(&config), json_of(want));
}

#[rstest]
#[case::json_capable("gemini-2.0-flash")]
#[case::older_model("gemini-1.5-pro")]
fn a_present_non_object_schema_blocks_the_rewrite(#[case] model: &str) {
    let given = r#"{"responseSchema":null,"response_schema":{"type":"object"}}"#;
    let mut config = object(given);
    normalize_response_schema(&mut config, model, &settings()).unwrap();
    assert_eq!(rendered(&config), json_of(given));
}

#[test]
fn without_a_schema_key_a_json_schema_alias_is_untouched() {
    let given = r#"{"response_json_schema":{"type":"object"},"temperature":0}"#;
    let mut config = object(given);
    normalize_response_schema(&mut config, OLD_MODEL, &settings()).unwrap();
    assert_eq!(rendered(&config), json_of(given));
}

#[test]
fn studio_request_always_carries_tools_and_generation_config() {
    let body = request(NativeProvider::Studio, MODEL, "{}");
    assert_eq!(
        rendered(&body),
        json_of(&format!(
            r#"{{"model":"{MODEL}","contents":{CONTENTS},"tools":null,"generationConfig":{{}}}}"#
        ))
    );
}

#[test]
fn studio_request_adds_a_system_instruction_for_a_non_null_value() {
    let request = GenerateContentRequest {
        model: MODEL.to_owned(),
        contents: json_of(CONTENTS),
        tools: json_of(r#"[{"functionDeclarations":[]}]"#),
        generation_config: object(r#"{"temperature":0}"#),
        system_instruction: json_of(r#"{"parts":[{"text":"behave"}]}"#),
    };
    let body = transform_request(NativeProvider::Studio, &request, &settings()).unwrap();
    assert_eq!(
        rendered(&body),
        json_of(&format!(
            r#"{{"model":"{MODEL}","contents":{CONTENTS},"tools":[{{"functionDeclarations":[]}}],"generationConfig":{{"temperature":0}},"systemInstruction":{{"parts":[{{"text":"behave"}}]}}}}"#
        ))
    );
}

#[test]
fn studio_request_leaves_out_a_null_instruction() {
    let request = GenerateContentRequest {
        model: MODEL.to_owned(),
        contents: json_of(CONTENTS),
        tools: Json::Null,
        generation_config: object("{}"),
        system_instruction: Json::Null,
    };
    let body = transform_request(NativeProvider::Studio, &request, &settings()).unwrap();
    assert_eq!(
        rendered(&body),
        json_of(&format!(
            r#"{{"model":"{MODEL}","contents":{CONTENTS},"tools":null,"generationConfig":{{}}}}"#
        ))
    );
}

#[rstest]
#[case::empty_list_and_object("[]", "{}")]
#[case::empty_text("false", r#""""#)]
#[case::zero("0", "[]")]
fn studio_request_keeps_falsy_tools_and_instruction(
    #[case] tools: &str,
    #[case] instruction: &str,
) {
    let request = GenerateContentRequest {
        model: MODEL.to_owned(),
        contents: json_of(CONTENTS),
        tools: json_of(tools),
        generation_config: object("{}"),
        system_instruction: json_of(instruction),
    };
    let body = transform_request(NativeProvider::Studio, &request, &settings()).unwrap();
    assert_eq!(
        rendered(&body),
        json_of(&format!(
            r#"{{"model":"{MODEL}","contents":{CONTENTS},"tools":{tools},"generationConfig":{{}},"systemInstruction":{instruction}}}"#
        ))
    );
}

#[rstest]
#[case::empty_tools("[]")]
#[case::null_tools("null")]
#[case::false_value("false")]
fn vertex_request_omits_a_falsy_tools_value(#[case] tools: &str) {
    let request = GenerateContentRequest {
        model: MODEL.to_owned(),
        contents: json_of(CONTENTS),
        tools: json_of(tools),
        generation_config: object(r#"{"temperature":0}"#),
        system_instruction: Json::Null,
    };
    let body = transform_request(NativeProvider::Vertex, &request, &settings()).unwrap();
    assert_eq!(
        rendered(&body),
        json_of(&format!(
            r#"{{"model":"{MODEL}","contents":{CONTENTS},"generationConfig":{{"temperature":0}}}}"#
        ))
    );
}

#[test]
fn vertex_request_omits_empty_config_and_instruction() {
    let request = GenerateContentRequest {
        model: MODEL.to_owned(),
        contents: json_of(CONTENTS),
        tools: Json::List(Vec::new()),
        generation_config: Object::new(),
        system_instruction: Json::Object(Object::new()),
    };
    let body = transform_request(NativeProvider::Vertex, &request, &settings()).unwrap();
    assert_eq!(
        rendered(&body),
        json_of(&format!(r#"{{"model":"{MODEL}","contents":{CONTENTS}}}"#))
    );
}

#[test]
fn vertex_request_keeps_truthy_fields_in_source_order() {
    let request = GenerateContentRequest {
        model: MODEL.to_owned(),
        contents: json_of(CONTENTS),
        tools: json_of(r#"[{"functionDeclarations":[]}]"#),
        generation_config: object(r#"{"temperature":0}"#),
        system_instruction: json_of(r#"{"parts":[{"text":"x"}]}"#),
    };
    let body = transform_request(NativeProvider::Vertex, &request, &settings()).unwrap();
    assert_eq!(
        rendered(&body),
        json_of(&format!(
            r#"{{"model":"{MODEL}","contents":{CONTENTS},"tools":[{{"functionDeclarations":[]}}],"systemInstruction":{{"parts":[{{"text":"x"}}]}},"generationConfig":{{"temperature":0}}}}"#
        ))
    );
}

#[test]
fn a_request_transform_leaves_the_caller_config_alone() {
    let given = r#"{"responseSchema":{"type":"object","properties":{"a":{"type":"string"}}}}"#;
    let request = GenerateContentRequest {
        model: OLD_MODEL.to_owned(),
        contents: json_of(CONTENTS),
        tools: Json::Null,
        generation_config: object(given),
        system_instruction: Json::Null,
    };
    transform_request(NativeProvider::Studio, &request, &settings()).unwrap();
    assert_eq!(rendered(&request.generation_config), json_of(given));
}

#[test]
fn response_citations_replace_existing_ones() {
    let response = response_of(
        r#"{"candidates":[{"citationMetadata":{"citationSources":[{"uri":"https://a.example"}],"citations":[{"uri":"stale"}],"extra":"kept"},"finishReason":"STOP"}]}"#,
    );
    assert_eq!(
        rendered(&response),
        json_of(
            r#"{"candidates":[{"citationMetadata":{"citations":[{"uri":"https://a.example"}],"extra":"kept"},"finishReason":"STOP"}]}"#
        )
    );
}

#[test]
fn response_citation_metadata_that_is_not_an_object_is_kept() {
    let source = r#"{"candidates":[{"citationMetadata":"not-an-object"}]}"#;
    assert_eq!(rendered(&response_of(source)), json_of(source));
}

#[test]
fn response_keeps_unknown_top_level_and_candidate_fields() {
    let source = r#"{"candidates":[{"xUnknown":{"deep":[1,2]},"finishReason":"STOP"}],"unknownTop":{"a":null},"modelVersion":"gemini-2.5-flash"}"#;
    assert_eq!(rendered(&response_of(source)), json_of(source));
}

#[rstest]
#[case::null_candidates(r#"{"candidates":null}"#)]
#[case::null_content(r#"{"candidates":[{"content":null}]}"#)]
#[case::null_reasons(
    r#"{"candidates":[{"finishReason":null,"index":null,"citationMetadata":null}]}"#
)]
#[case::null_top_level(
    r#"{"promptFeedback":null,"usageMetadata":null,"responseId":null,"modelVersion":null}"#
)]
#[case::candidates_object(r#"{"candidates":{"a":1}}"#)]
#[case::candidates_number(r#"{"candidates":5}"#)]
#[case::candidates_bool(r#"{"candidates":true}"#)]
#[case::candidate_number(r#"{"candidates":[7]}"#)]
#[case::candidate_null(r#"{"candidates":[null]}"#)]
#[case::content_list(r#"{"candidates":[{"content":[]}]}"#)]
#[case::non_integer_index(r#"{"candidates":[{"index":"first"}]}"#)]
#[case::response_id(r#"{"responseId":7}"#)]
fn response_keeps_every_field_it_cannot_widen(#[case] source: &str) {
    assert_eq!(rendered(&response_of(source)), json_of(source));
}

#[test]
fn decoded_candidates_use_the_shared_records() {
    let response = response_of(
        r#"{"candidates":[{"content":{"parts":[{"text":"hello"},{"thought":true}]},"finishReason":"STOP","xUnknown":1}]}"#,
    );
    let candidates = response.candidates().unwrap();
    let candidate = candidates.first().unwrap();
    let parts = candidate.content.as_ref().unwrap().parts.as_ref().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(
        parts.first().and_then(|part| part.text.as_deref()),
        Some("hello")
    );
    assert_eq!(candidate.finish_reason.as_deref(), Some("STOP"));
    assert!(candidate.metadata.contains_key("xUnknown"));
}

#[test]
fn decoded_usage_uses_the_shared_record() {
    let response = response_of(
        r#"{"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5}}"#,
    );
    let usage = response.usage().unwrap().unwrap();
    assert_eq!(usage.prompt_token_count, 3);
    assert_eq!(usage.candidates_token_count, 2);
    assert_eq!(usage.total_token_count, 5);
}

#[test]
fn malformed_response_json_is_an_error() {
    let error = decode_google_native_content(b"not json").unwrap_err();
    assert!(matches!(error, NativeContentError::Json { .. }));
}

#[test]
fn non_standard_number_literals_are_refused() {
    let error = decode_google_native_content(br#"{"value":NaN}"#).unwrap_err();
    assert!(matches!(error, NativeContentError::Json { .. }));
}

#[test]
fn a_response_nested_past_the_depth_limit_is_refused() {
    let mut body = String::new();
    body.push_str(&r#"{"a":"#.repeat(200));
    body.push('1');
    body.push_str(&"}".repeat(200));
    let error = decode_google_native_content(body.as_bytes()).unwrap_err();
    assert!(matches!(error, NativeContentError::Json { .. }));
}

#[test]
fn a_non_object_response_root_is_refused() {
    let error = decode_google_native_content(b"[1,2]").unwrap_err();
    assert!(matches!(error, NativeContentError::ResponseNotObject));
}

#[test]
fn an_oversized_response_is_refused() {
    let bytes = vec![b' '; MAX_RESPONSE_BYTES + 1];
    let error = decode_google_native_content(&bytes).unwrap_err();
    assert!(matches!(
        error,
        NativeContentError::ResponseTooLarge { limit, .. } if limit == MAX_RESPONSE_BYTES
    ));
}

#[test]
fn a_response_at_the_size_limit_is_accepted() {
    let mut body = String::with_capacity(MAX_RESPONSE_BYTES);
    body.push_str("{\"pad\":\"");
    body.push_str(&"a".repeat(MAX_RESPONSE_BYTES - body.len() - 2));
    body.push_str("\"}");
    assert_eq!(body.len(), MAX_RESPONSE_BYTES);
    let response = decode_google_native_content(body.as_bytes()).unwrap();
    assert!(response.wire.contains_key("pad"));
}

#[test]
fn the_transform_retains_the_raw_status_and_headers() {
    let mut headers = indexmap::IndexMap::new();
    headers.insert("x-request-id".to_owned(), "r1".to_owned());
    let raw = RawResponse {
        status: 200,
        headers: &headers,
        body: br#"{"candidates":[]}"#,
    };
    let native = transform_response(&raw).unwrap();
    assert_eq!(native.status, 200);
    assert_eq!(native.headers, headers);
    assert!(native.response.candidates().unwrap().is_empty());
}

#[test]
fn a_transform_error_carries_the_status_and_headers() {
    let mut headers = indexmap::IndexMap::new();
    headers.insert("content-type".to_owned(), "text/html".to_owned());
    let raw = RawResponse {
        status: 502,
        headers: &headers,
        body: b"<html>bad gateway</html>",
    };
    let error = transform_response(&raw).unwrap_err();
    assert!(matches!(
        error,
        NativeContentError::Response { status: 502, headers: carried, .. } if carried == headers
    ));
}

fn studio_settings(environment_key: Option<&str>, generic_key: Option<&str>) -> StudioSettings {
    StudioSettings {
        environment_key: environment_key.map(secret),
        generic_api_key: generic_key.map(secret),
        api_base: DEFAULT_API_BASE.to_owned(),
    }
}

fn vertex_params<'params>(
    project: Option<&'params str>,
    location: Option<&'params str>,
) -> VertexParams<'params> {
    VertexParams {
        project,
        ai_project: None,
        location,
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    }
}

fn request_key(text: &str) -> NativeParams {
    NativeParams {
        api_key: Some(AuthValue::Key(secret(text))),
        gemini_api_key: None,
    }
}

#[tokio::test]
async fn studio_auth_uses_the_request_key_and_never_reads_credentials() {
    let studio = studio_settings(None, None);
    let vertex = VertexSettings::default();
    let catalog = catalog();
    let auth = auth(
        NativeProvider::Studio,
        vertex_params(None, None),
        &vertex,
        &studio,
        request_key("request-key"),
    );
    // a Gemini call must never poll the credential lookup, so its error never surfaces
    let target = auth_token_and_url(&catalog, &auth, |_, _| {
        ready(Err(CredentialCacheError::MissingToken))
    })
    .await
    .unwrap();
    assert_eq!(
        target.url,
        format!("https://generativelanguage.googleapis.com/v1beta/models/{MODEL}:generateContent")
    );
    assert_eq!(header(&target, "Content-Type"), Some("application/json"));
    assert_eq!(header(&target, X_GOOGLE_API_KEY), Some("request-key"));
}

#[tokio::test]
async fn studio_auth_prefers_the_request_key_over_the_environment_key() {
    let studio = studio_settings(Some("environment-key"), None);
    let vertex = VertexSettings::default();
    let catalog = catalog();
    let auth = auth(
        NativeProvider::Studio,
        vertex_params(None, None),
        &vertex,
        &studio,
        request_key("request-key"),
    );
    let target = auth_token_and_url(&catalog, &auth, |_, _| ready(Ok(access_token("", ""))))
        .await
        .unwrap();
    assert_eq!(header(&target, X_GOOGLE_API_KEY), Some("request-key"));
}

#[tokio::test]
async fn studio_auth_falls_back_to_the_environment_key() {
    let studio = studio_settings(Some("environment-key"), Some("generic-key"));
    let vertex = VertexSettings::default();
    let catalog = catalog();
    let auth = auth(
        NativeProvider::Studio,
        vertex_params(None, None),
        &vertex,
        &studio,
        NativeParams::default(),
    );
    let target = auth_token_and_url(&catalog, &auth, |_, _| ready(Ok(access_token("", ""))))
        .await
        .unwrap();
    assert_eq!(header(&target, X_GOOGLE_API_KEY), Some("environment-key"));
}

#[tokio::test]
async fn studio_auth_prefers_the_gemini_key_after_the_plain_key() {
    let studio = studio_settings(None, Some("generic-key"));
    let vertex = VertexSettings::default();
    let catalog = catalog();
    let keys = NativeParams {
        api_key: None,
        gemini_api_key: Some(AuthValue::Key(secret("gemini-key"))),
    };
    let auth = auth(
        NativeProvider::Studio,
        vertex_params(None, None),
        &vertex,
        &studio,
        keys,
    );
    let target = auth_token_and_url(&catalog, &auth, |_, _| ready(Ok(access_token("", ""))))
        .await
        .unwrap();
    assert_eq!(header(&target, X_GOOGLE_API_KEY), Some("gemini-key"));
}

#[tokio::test]
async fn studio_auth_refuses_a_key_that_is_missing_or_not_a_string() {
    let studio = studio_settings(None, None);
    let vertex = VertexSettings::default();
    let catalog = catalog();
    let missing = auth(
        NativeProvider::Studio,
        vertex_params(None, None),
        &vertex,
        &studio,
        NativeParams::default(),
    );
    let error = auth_token_and_url(&catalog, &missing, |_, _| ready(Ok(access_token("", ""))))
        .await
        .unwrap_err();
    assert!(matches!(error, NativeContentError::StudioKey));
    let keys = NativeParams {
        api_key: Some(AuthValue::Headers(BTreeMap::from([(
            X_GOOGLE_API_KEY.to_owned(),
            secret("map-key"),
        )]))),
        gemini_api_key: None,
    };
    let mapped = auth(
        NativeProvider::Studio,
        vertex_params(None, None),
        &vertex,
        &studio,
        keys,
    );
    let header_error =
        auth_token_and_url(&catalog, &mapped, |_, _| ready(Ok(access_token("", ""))))
            .await
            .unwrap_err();
    assert!(matches!(header_error, NativeContentError::StudioKey));
}

#[tokio::test]
async fn vertex_auth_uses_the_resolved_project_and_a_bearer_token() {
    let studio = studio_settings(None, None);
    let vertex = VertexSettings::default();
    let catalog = catalog();
    let auth = auth(
        NativeProvider::Vertex,
        vertex_params(Some("configured-project"), Some("us-central1")),
        &vertex,
        &studio,
        NativeParams::default(),
    );
    let target = auth_token_and_url(&catalog, &auth, |credentials, project| {
        assert!(credentials.is_none());
        assert_eq!(project, Some("configured-project"));
        ready(Ok(access_token("access-token", "resolved-project")))
    })
    .await
    .unwrap();
    assert_eq!(
        target.url,
        format!(
            "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/resolved-project/locations/us-central1/publishers/google/models/{MODEL}:generateContent"
        )
    );
    assert_eq!(header(&target, "Content-Type"), Some("application/json"));
    assert_eq!(
        header(&target, "Authorization"),
        Some("Bearer access-token")
    );
}

#[tokio::test]
async fn vertex_auth_streams_with_alt_sse() {
    let studio = studio_settings(None, None);
    let vertex = VertexSettings::default();
    let catalog = catalog();
    let mut auth = auth(
        NativeProvider::Vertex,
        vertex_params(Some("proj"), Some("us-central1")),
        &vertex,
        &studio,
        NativeParams::default(),
    );
    auth.stream = true;
    let target = auth_token_and_url(&catalog, &auth, |_, _| {
        ready(Ok(access_token("tok", "proj")))
    })
    .await
    .unwrap();
    assert!(target.url.ends_with(":streamGenerateContent?alt=sse"));
}

#[tokio::test]
async fn vertex_auth_takes_the_project_and_location_from_the_settings() {
    let studio = studio_settings(None, None);
    let vertex = VertexSettings {
        project: Some("setting-project".to_owned()),
        location: Some("europe-west4".to_owned()),
        credentials: None,
        adc: AdcEnvironment::default(),
    };
    let catalog = catalog();
    let auth = auth(
        NativeProvider::Vertex,
        vertex_params(None, None),
        &vertex,
        &studio,
        NativeParams::default(),
    );
    let target = auth_token_and_url(&catalog, &auth, |credentials, project| {
        assert!(credentials.is_none());
        assert_eq!(project, Some("setting-project"));
        ready(Ok(access_token("tok", project.unwrap_or_default())))
    })
    .await
    .unwrap();
    assert!(
        target
            .url
            .starts_with("https://europe-west4-aiplatform.googleapis.com/")
    );
    assert!(target.url.contains("/projects/setting-project/"));
}

#[tokio::test]
async fn vertex_auth_propagates_a_token_error() {
    let studio = studio_settings(None, None);
    let vertex = VertexSettings::default();
    let catalog = catalog();
    let auth = auth(
        NativeProvider::Vertex,
        vertex_params(Some("proj"), Some("us-central1")),
        &vertex,
        &studio,
        NativeParams::default(),
    );
    let error = auth_token_and_url(&catalog, &auth, |_, _| async {
        Err(CredentialCacheError::MissingToken)
    })
    .await
    .unwrap_err();
    assert!(matches!(error, NativeContentError::Credentials { .. }));
}

#[rstest]
#[case::missing(None, None, None)]
#[case::empty(Some(""), None, Some("Bearer "))]
#[case::present(Some("token"), None, Some("Bearer token"))]
#[case::caller_wins(Some("token"), Some("caller"), Some("caller"))]
fn vertex_header_validation_adds_bearer_only_for_a_present_key(
    #[case] token: Option<&str>,
    #[case] caller_token: Option<&str>,
    #[case] want: Option<&str>,
) {
    let key = token.map(secret);
    let caller = caller_token.map_or_else(BTreeMap::new, |value| {
        BTreeMap::from([("Authorization".to_owned(), value.to_owned())])
    });
    let headers = set_headers(key.as_ref(), &caller);
    assert_eq!(
        headers
            .get("Authorization")
            .map(SecretString::expose_secret),
        want
    );
    assert_eq!(
        headers.get("Content-Type").map(SecretString::expose_secret),
        Some("application/json")
    );
}

#[rstest]
fn replays_the_google_native_content_corpus(
    #[files("fuzz/corpus/google_native_content/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_google_native_content(&bytes));
    let headers = indexmap::IndexMap::new();
    let raw = RawResponse {
        status: 200,
        headers: &headers,
        body: &bytes,
    };
    drop(transform_response(&raw));
}
