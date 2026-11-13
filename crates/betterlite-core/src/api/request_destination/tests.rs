#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use tracing::subscriber::{DefaultGuard, set_default};

use super::DestinationSettings;
use super::RequestData;
use super::RequestDestinationInput;
use super::RequestDestinationResult;
use super::destination::{
    DestinationField, reject_url_valued_destination, reject_url_valued_destinations,
};
use super::error::{ErrorFacts, ExceptionDetail, RequestDestinationError};
use super::model::{InferenceModelKind, ModelDefaults, resolve_inference_model};
use super::parameters::{populate_request_parameters, tags_from_request_body};
use super::resolve_request_destination;

fn data(source: &str) -> RequestData {
    serde_json::from_str(source).unwrap()
}

fn defaults(
    completion_model: Option<&str>,
    image_generation_model: Option<&str>,
    moderation_model: Option<&str>,
) -> ModelDefaults {
    ModelDefaults {
        completion_model: completion_model.map(str::to_owned),
        image_generation_model: image_generation_model.map(str::to_owned),
        moderation_model: moderation_model.map(str::to_owned),
    }
}

fn hosts(entries: &[&str]) -> Vec<AllowedDestination> {
    entries
        .iter()
        .filter_map(|entry| AllowedDestination::parse(entry))
        .collect()
}

fn query_params(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

struct Precedence {
    kind: InferenceModelKind,
    defaults: ModelDefaults,
    cli_model: Option<&'static str>,
    endpoint_model: Option<&'static str>,
    body_model: Option<&'static str>,
}

fn precedence(
    kind: InferenceModelKind,
    defaults: ModelDefaults,
    cli_model: Option<&'static str>,
    endpoint_model: Option<&'static str>,
    body_model: Option<&'static str>,
) -> Precedence {
    Precedence {
        kind,
        defaults,
        cli_model,
        endpoint_model,
        body_model,
    }
}

#[rstest]
#[case::completion_settings_first(
    precedence(
        InferenceModelKind::Completion,
        defaults(Some("default"), None, None),
        Some("cli"),
        Some("path"),
        Some("body")
    ),
    Some("default")
)]
#[case::completion_cli_second(
    precedence(
        InferenceModelKind::Completion,
        defaults(None, None, None),
        Some("cli"),
        Some("path"),
        Some("body")
    ),
    Some("cli")
)]
#[case::completion_endpoint_third(
    precedence(
        InferenceModelKind::Completion,
        defaults(None, None, None),
        None,
        Some("path"),
        Some("body")
    ),
    Some("path")
)]
#[case::completion_body_last(
    precedence(
        InferenceModelKind::Completion,
        defaults(None, None, None),
        None,
        None,
        Some("body")
    ),
    Some("body")
)]
#[case::completion_without_any_source(
    precedence(
        InferenceModelKind::Completion,
        defaults(None, None, None),
        None,
        None,
        None
    ),
    None
)]
#[case::image_generation_skips_completion_model(
    precedence(
        InferenceModelKind::ImageGeneration,
        defaults(Some("text"), Some("image"), None),
        None,
        None,
        Some("body")
    ),
    Some("image")
)]
#[case::image_generation_cli_first(
    precedence(
        InferenceModelKind::ImageGeneration,
        defaults(None, Some("image"), None),
        Some("cli"),
        Some("path"),
        Some("body")
    ),
    Some("cli")
)]
#[case::image_generation_endpoint_second(
    precedence(
        InferenceModelKind::ImageGeneration,
        defaults(None, Some("image"), None),
        None,
        Some("path"),
        Some("body")
    ),
    Some("path")
)]
#[case::image_generation_body_last(
    precedence(
        InferenceModelKind::ImageGeneration,
        defaults(None, Some("image"), None),
        None,
        None,
        None
    ),
    Some("image")
)]
#[case::image_edit_starts_with_completion_model(
    precedence(
        InferenceModelKind::ImageEdit,
        defaults(Some("text"), Some("image"), None),
        None,
        None,
        Some("body")
    ),
    Some("text")
)]
#[case::image_edit_endpoint_before_image_model(
    precedence(
        InferenceModelKind::ImageEdit,
        defaults(None, Some("image"), None),
        None,
        Some("path"),
        Some("body")
    ),
    Some("path")
)]
#[case::image_edit_falls_to_image_model(
    precedence(
        InferenceModelKind::ImageEdit,
        defaults(None, Some("image"), None),
        None,
        None,
        Some("body")
    ),
    Some("image")
)]
#[case::moderation_cli_first(
    precedence(
        InferenceModelKind::Moderation,
        defaults(None, None, Some("mod")),
        Some("cli"),
        None,
        Some("body")
    ),
    Some("cli")
)]
#[case::moderation_default_second(
    precedence(
        InferenceModelKind::Moderation,
        defaults(None, None, Some("mod")),
        None,
        None,
        Some("body")
    ),
    Some("mod")
)]
#[case::speech_ignores_defaults(
    precedence(
        InferenceModelKind::Speech,
        defaults(Some("text"), None, None),
        None,
        None,
        Some("body")
    ),
    Some("body")
)]
#[case::body_ignores_every_other_source(
    precedence(
        InferenceModelKind::Body,
        defaults(Some("text"), None, None),
        Some("cli"),
        None,
        Some("body")
    ),
    Some("body")
)]
#[case::path_takes_the_endpoint(
    precedence(
        InferenceModelKind::Path,
        defaults(Some("text"), None, None),
        Some("cli"),
        Some("path"),
        Some("body")
    ),
    Some("path")
)]
#[case::explicit_empty_completion_model_is_used(
    precedence(
        InferenceModelKind::Completion,
        defaults(Some(""), None, None),
        Some("cli"),
        None,
        Some("body")
    ),
    Some("")
)]
#[case::explicit_empty_body_model_is_used(
    precedence(
        InferenceModelKind::Body,
        defaults(None, None, None),
        Some("cli"),
        None,
        Some("")
    ),
    Some("")
)]
fn endpoint_model_precedence(#[case] case: Precedence, #[case] expected: Option<&str>) {
    let resolved = resolve_inference_model(
        case.body_model,
        &case.defaults,
        case.cli_model,
        case.endpoint_model,
        case.kind,
    );
    assert_eq!(resolved, expected.map(str::to_owned));
}

#[rstest]
#[case::plain_model(DestinationField::Model, "gpt-4", &[], false)]
#[case::plain_file_id(DestinationField::FileId, "files/abc123", &[], false)]
#[case::empty_value(DestinationField::Model, "", &[], false)]
#[case::url_valued_model(DestinationField::Model, "https://attacker.example/v1", &[], true)]
#[case::url_valued_file_id(DestinationField::FileId, "https://attacker.example/v1beta/files/abc", &[], true)]
#[case::http_scheme(DestinationField::Model, "http://10.0.0.1:8080/v1", &[], true)]
#[case::provider_prefixed_url(DestinationField::Model, "huggingface/https://attacker.example/v1", &[], true)]
#[case::comma_smuggled_url(DestinationField::Model, "gpt-4,huggingface/https://attacker.example/v1", &[], true)]
#[case::uppercase_scheme(DestinationField::Model, "huggingface/HTTPS://evil.example/v1", &[], true)]
#[case::provider_prefixed_plain_model(DestinationField::Model, "huggingface/BAAI/bge-small-en", &[], false)]
#[case::comma_batch_of_plain_models(DestinationField::Model, "gpt-4,huggingface/BAAI/bge-small-en", &[], false)]
#[case::allowlisted_host(DestinationField::Model, "https://trusted.example/v1", &["trusted.example"], false)]
#[case::allowlisted_origin_accepts_its_scheme(DestinationField::Model, "https://trusted.example/v1", &["https://trusted.example"], false)]
#[case::allowlisted_origin_rejects_other_scheme(DestinationField::Model, "http://trusted.example/v1", &["https://trusted.example"], true)]
#[case::allowlisted_port_accepts_its_port(DestinationField::Model, "https://trusted.example:8443/v1", &["trusted.example:8443"], false)]
#[case::allowlisted_port_rejects_other_port(DestinationField::Model, "https://trusted.example/v1", &["trusted.example:8443"], true)]
#[case::userinfo_is_rejected_even_when_allowlisted(DestinationField::Model, "https://user:pass@trusted.example/v1", &["trusted.example"], true)]
#[case::host_matches_case_insensitively(DestinationField::Model, "HTTPS://TRUSTED.EXAMPLE./v1", &["trusted.example"], false)]
fn deployment_url_rejected(
    #[case] field: DestinationField,
    #[case] value: &str,
    #[case] allowed_hosts: &[&str],
    #[case] rejected: bool,
) {
    let allowed = hosts(allowed_hosts);
    let outcome = reject_url_valued_destination(field, value, &allowed);
    if !rejected {
        assert_eq!(outcome, Ok(()));
        return;
    }
    let error = outcome.expect_err("a URL-valued destination is rejected");
    assert_eq!(
        error,
        RequestDestinationError::UrlValuedDestination { field }
    );
    assert_eq!(
        error.facts(),
        ErrorFacts {
            status: 400,
            error_type: "invalid_request_error",
            param: None,
            code: "400",
            detail: Some(ExceptionDetail {
                error: "invalid_request",
                param: field,
            }),
        }
    );
}

#[test]
fn an_allowed_destination_passes() {
    let allowed = hosts(&["trusted.example"]);
    assert_eq!(
        reject_url_valued_destination(
            DestinationField::Model,
            "https://trusted.example/v1",
            &allowed
        ),
        Ok(())
    );
}

#[test]
fn data_destination_fields_are_checked_in_order() {
    let body = data(r#"{"model":"https://one.example/v1","file_id":"https://two.example/v1"}"#);
    assert_eq!(
        reject_url_valued_destinations(&body, &[]),
        Err(RequestDestinationError::UrlValuedDestination {
            field: DestinationField::Model,
        })
    );
    let only_file_id = data(r#"{"file_id":"https://two.example/v1"}"#);
    assert_eq!(
        reject_url_valued_destinations(&only_file_id, &[]),
        Err(RequestDestinationError::UrlValuedDestination {
            field: DestinationField::FileId,
        })
    );
}

#[test]
fn non_text_destination_values_are_left_alone() {
    let body = data(r#"{"model":42,"file_id":["https://two.example/v1"]}"#);
    assert_eq!(reject_url_valued_destinations(&body, &[]), Ok(()));
}

#[test]
fn destination_rejection_precedes_model_resolution() {
    let request = RequestDestinationInput {
        data: data(r#"{"model":"https://attacker.example/v1"}"#),
        kind: InferenceModelKind::Body,
        ..body_input()
    };
    assert_eq!(
        resolve_request_destination(&request, &DestinationSettings::default()),
        Err(RequestDestinationError::UrlValuedDestination {
            field: DestinationField::Model,
        })
    );
}

#[test]
fn the_route_endpoint_model_is_rejected() {
    let request = RequestDestinationInput {
        endpoint_model: Some("https://attacker.example/v1".to_owned()),
        ..body_input()
    };
    assert_eq!(
        resolve_request_destination(&request, &DestinationSettings::default()),
        Err(RequestDestinationError::UrlValuedDestination {
            field: DestinationField::Model,
        })
    );
}

fn body_input() -> RequestDestinationInput {
    RequestDestinationInput {
        data: data(r#"{"messages":[]}"#),
        path_params: RequestData::new(),
        query_params: IndexMap::new(),
        route_path: "/v1/chat/completions".to_owned(),
        kind: InferenceModelKind::Body,
        cli_model: None,
        endpoint_model: None,
    }
}

#[rstest]
#[case::completion(InferenceModelKind::Completion)]
#[case::image_generation(InferenceModelKind::ImageGeneration)]
#[case::image_edit(InferenceModelKind::ImageEdit)]
#[case::moderation(InferenceModelKind::Moderation)]
#[case::speech(InferenceModelKind::Speech)]
#[case::body(InferenceModelKind::Body)]
#[case::path(InferenceModelKind::Path)]
fn missing_model_source_error(#[case] kind: InferenceModelKind) {
    let request = RequestDestinationInput {
        kind,
        ..body_input()
    };
    let result = resolve_request_destination(&request, &DestinationSettings::default())
        .expect("a request without a model source still resolves a destination");
    assert_eq!(result.model, None);
    assert_eq!(result.data.get("model"), Some(&Json::Null));
    assert_eq!(result.tags, Vec::<String>::new());
}

#[test]
fn a_configured_default_fills_the_missing_model() {
    let settings = DestinationSettings {
        model_defaults: defaults(Some("default"), None, None),
        allowed_hosts: Vec::new(),
    };
    let request = RequestDestinationInput {
        kind: InferenceModelKind::Completion,
        ..body_input()
    };
    let result = resolve_request_destination(&request, &settings).unwrap();
    assert_eq!(
        result,
        RequestDestinationResult {
            data: data(r#"{"messages":[],"model":"default"}"#),
            model: Some("default".to_owned()),
            tags: Vec::new(),
        }
    );
}

#[test]
fn a_non_text_model_is_rejected() {
    let request = RequestDestinationInput {
        data: data(r#"{"model":42}"#),
        ..body_input()
    };
    let error = resolve_request_destination(&request, &DestinationSettings::default())
        .expect_err("a non-string model is rejected");
    assert_eq!(error, RequestDestinationError::ModelNotText);
    assert_eq!(
        error.facts(),
        ErrorFacts {
            status: 400,
            error_type: "bad_request_error",
            param: Some("model"),
            code: "400",
            detail: None,
        }
    );
}

#[rstest]
#[case::object_metadata_and_body_tags(r#"{"metadata":{"tags":["a"]},"tags":["b"]}"#, &["a", "b"])]
#[case::metadata_json_string(r#"{"metadata":"{\"tags\":[\"a\"]}","tags":["b"]}"#, &["a", "b"])]
#[case::metadata_json_string_not_an_object(r#"{"metadata":"[1,2]","tags":["b"]}"#, &["b"])]
#[case::invalid_metadata_string(r#"{"metadata":"not json","tags":["b"]}"#, &["b"])]
#[case::metadata_not_an_object(r#"{"metadata":[1],"tags":["b"]}"#, &["b"])]
#[case::non_string_tags_are_dropped(r#"{"metadata":{"tags":["a",1]},"tags":["b",true]}"#, &["a", "b"])]
#[case::tags_must_be_lists(r#"{"metadata":{"tags":"a"},"tags":"b"}"#, &[])]
#[case::duplicates_are_kept(r#"{"metadata":{"tags":["a","a"]},"tags":["a","b"]}"#, &["a", "a", "a", "b"])]
#[case::litellm_metadata_wins_over_metadata(
    r#"{"litellm_metadata":{"tags":["inner"]},"metadata":{"tags":["outer"]},"tags":["body"]}"#,
    &["inner", "body"]
)]
#[case::null_litellm_metadata_hides_metadata(
    r#"{"litellm_metadata":null,"metadata":{"tags":["outer"]},"tags":["body"]}"#,
    &["body"]
)]
fn tags_from_metadata_and_body(#[case] source: &str, #[case] expected: &[&str]) {
    assert_eq!(tags_from_request_body(&data(source)), expected);
}

#[test]
fn query_and_path_parameters_fill_missing_keys() {
    let body = data(r#"{"model":"m"}"#);
    let path_params = data(r#"{"vector_store_id":"vs_from_path","other":"path"}"#);
    let populated = populate_request_parameters(
        &body,
        &path_params,
        &query_params(&[("model", "query"), ("limit", "2")]),
        "/v1/chat/completions",
    );
    assert_eq!(
        populated,
        data(
            r#"{"model":"m","limit":"2","vector_store_id":"vs_from_path","other":"path","vector_store_ids":["vs_from_path"]}"#
        )
    );
}

#[test]
fn body_values_win_over_query_and_path_values() {
    let body =
        data(r#"{"model":"body","vector_store_id":"body_id","vector_store_ids":["body_id"]}"#);
    let path_params = data(r#"{"vector_store_id":"path_id"}"#);
    let populated = populate_request_parameters(
        &body,
        &path_params,
        &query_params(&[("model", "query")]),
        "/v1/chat/completions",
    );
    assert_eq!(
        populated,
        data(
            r#"{"model":"body","vector_store_id":"body_id","vector_store_ids":["body_id","path_id"]}"#
        )
    );
}

#[test]
fn vector_store_ids_grow_from_the_route_path() {
    let body = data(r#"{"vector_store_ids":["already"]}"#);
    let populated = populate_request_parameters(
        &body,
        &RequestData::new(),
        &IndexMap::new(),
        "/v1/vector_stores/vs_123/files",
    );
    assert_eq!(
        populated,
        data(r#"{"vector_store_ids":["already","vs_123"],"vector_store_id":"vs_123"}"#)
    );
    let present = data(r#"{"vector_store_ids":["vs_123"]}"#);
    let without_duplicates = populate_request_parameters(
        &present,
        &RequestData::new(),
        &IndexMap::new(),
        "/v1/vector_stores/vs_123/files",
    );
    assert_eq!(
        without_duplicates,
        data(r#"{"vector_store_ids":["vs_123"],"vector_store_id":"vs_123"}"#)
    );
    let empty_params = RequestData::new();
    let from_the_route = populate_request_parameters(
        &empty_params,
        &RequestData::new(),
        &IndexMap::new(),
        "/v1/vector_stores/vs_123/files",
    );
    assert_eq!(
        from_the_route,
        data(r#"{"vector_store_ids":["vs_123"],"vector_store_id":"vs_123"}"#)
    );
}

#[rstest]
#[case::with_id_and_trailing_slash("/v1/vector_stores/vs_123/files", Some("vs_123"))]
#[case::without_trailing_slash("/v1/vector_stores/vs_123", None)]
#[case::empty_segment_before_a_match("/v1/vector_stores//vector_stores/vs_9/files", Some("vs_9"))]
#[case::first_match_wins("/v1/vector_stores/a/b/vector_stores/c/d", Some("a"))]
#[case::no_vector_store_route("/v1/chat/completions", None)]
fn vector_store_id_from_the_route(#[case] route_path: &str, #[case] expected_id: Option<&str>) {
    let populated = populate_request_parameters(
        &RequestData::new(),
        &RequestData::new(),
        &IndexMap::new(),
        route_path,
    );
    let expected = expected_id.map(|id| Json::Text(id.to_owned()));
    assert_eq!(populated.get("vector_store_id"), expected.as_ref());
}

#[test]
fn a_non_list_vector_store_ids_value_is_replaced() {
    let body = data(r#"{"vector_store_ids":"not-a-list"}"#);
    let path_params = data(r#"{"vector_store_id":"vs_1"}"#);
    let populated = populate_request_parameters(
        &body,
        &path_params,
        &IndexMap::new(),
        "/v1/chat/completions",
    );
    assert_eq!(
        populated,
        data(r#"{"vector_store_ids":["vs_1"],"vector_store_id":"vs_1"}"#)
    );
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_debug() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

#[test]
fn populate_keeps_its_debug_diagnostics() {
    let (captured, guard) = capture_debug();
    let path_params = data(r#"{"vector_store_id":"vs_1"}"#);
    drop(populate_request_parameters(
        &RequestData::new(),
        &path_params,
        &IndexMap::new(),
        "/v1/chat/completions",
    ));
    drop(populate_request_parameters(
        &RequestData::new(),
        &RequestData::new(),
        &IndexMap::new(),
        "/v1/vector_stores/vs_2/files",
    ));
    drop(populate_request_parameters(
        &RequestData::new(),
        &RequestData::new(),
        &IndexMap::new(),
        "/v1/chat/completions",
    ));
    drop(guard);
    let text = captured.text();
    assert!(text.contains("Found path_params"));
    assert!(text.contains("Extracted vector_store_id=vs_2"));
    assert!(text.contains("No vector_store_id present in path=/v1/chat/completions"));
}
