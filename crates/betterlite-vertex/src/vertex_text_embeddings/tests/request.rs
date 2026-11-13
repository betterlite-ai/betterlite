use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingInput;
use rstest::rstest;

use super::support::json;
use super::support::object;
use super::support::text;
use crate::vertex_text_embeddings::error::VertexTextEmbeddingError;
use crate::vertex_text_embeddings::request::MAPPED_SPECIAL_AUTH_PARAMS;
use crate::vertex_text_embeddings::request::SUPPORTED_OPENAI_PARAMS;
use crate::vertex_text_embeddings::request::map_openai_params;
use crate::vertex_text_embeddings::request::map_special_auth_params;
use crate::vertex_text_embeddings::request::transform_request;

fn mapped(options: &str, supplied: &str, extra: &str) -> (Json, Json) {
    let (mapped_optional, mapped_kwargs) =
        map_openai_params(&object(options), object(supplied), object(extra));
    (Json::Object(mapped_optional), Json::Object(mapped_kwargs))
}

#[rstest]
#[case::empty("{}", "{}", "{}", "{}")]
#[case::dimensions(r#"{"dimensions": 4}"#, "{}", "{}", r#"{"outputDimensionality": 4}"#)]
#[case::dimensions_overwrite(
    r#"{"dimensions": 4}"#,
    r#"{"outputDimensionality": 8}"#,
    "{}",
    r#"{"outputDimensionality": 4}"#
)]
#[case::dimensions_null(
    r#"{"dimensions": null}"#,
    "{}",
    "{}",
    r#"{"outputDimensionality": null}"#
)]
#[case::other_dropped(r#"{"user": "me"}"#, "{}", "{}", "{}")]
#[case::optional_kept(
    r#"{"dimensions": 4}"#,
    r#"{"auto_truncate": true}"#,
    "{}",
    r#"{"auto_truncate": true, "outputDimensionality": 4}"#
)]
fn dimensions_map_to_output_dimensionality(
    #[case] options: &str,
    #[case] optional: &str,
    #[case] kwargs: &str,
    #[case] expected: &str,
) {
    let (mapped_optional, mapped_kwargs) = mapped(options, optional, kwargs);
    assert_eq!(mapped_optional, json(expected));
    assert_eq!(mapped_kwargs, json("{}"));
}

#[rstest]
#[case::input_type(
    r#"{"input_type": "RETRIEVAL_DOCUMENT"}"#,
    r#"{"task_type": "RETRIEVAL_DOCUMENT"}"#,
    "{}"
)]
#[case::input_type_null(r#"{"input_type": null}"#, r#"{"task_type": null}"#, "{}")]
#[case::other_kwargs_kept(
    r#"{"input_type": "CLUSTERING", "user": "me"}"#,
    r#"{"task_type": "CLUSTERING"}"#,
    r#"{"user": "me"}"#
)]
#[case::absent(r#"{"user": "me"}"#, "{}", r#"{"user": "me"}"#)]
fn input_type_moves_into_task_type(
    #[case] kwargs: &str,
    #[case] expected: &str,
    #[case] expected_kwargs: &str,
) {
    let (mapped_optional, mapped_kwargs) = mapped("{}", "{}", kwargs);
    assert_eq!(mapped_optional, json(expected));
    assert_eq!(mapped_kwargs, json(expected_kwargs));
}

#[rstest]
#[case::project(r#"{"project": "proj"}"#, r#"{"vertex_project": "proj"}"#)]
#[case::region(
    r#"{"region_name": "us-central1"}"#,
    r#"{"vertex_location": "us-central1"}"#
)]
#[case::both(
    r#"{"project": "proj", "region_name": "eu"}"#,
    r#"{"vertex_project": "proj", "vertex_location": "eu"}"#
)]
#[case::unknown(r#"{"model": "m"}"#, "{}")]
#[case::empty("{}", "{}")]
fn auth_aliases_map_to_vertex_names(#[case] options: &str, #[case] expected: &str) {
    let mapped = map_special_auth_params(&object(options), object("{}"));
    assert_eq!(Json::Object(mapped), json(expected));
}

#[test]
fn auth_aliases_keep_supplied_optional_params() {
    let mapped = map_special_auth_params(&object(r#"{"project": "proj"}"#), object(r#"{"a": 1}"#));
    assert_eq!(
        Json::Object(mapped),
        json(r#"{"a": 1, "vertex_project": "proj"}"#)
    );
}

#[test]
fn supported_openai_params_is_dimensions_only() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["dimensions"]);
    assert_eq!(
        MAPPED_SPECIAL_AUTH_PARAMS,
        [
            ("project", "vertex_project"),
            ("region_name", "vertex_location")
        ]
    );
}

fn request(input: &EmbeddingInput, optional: &str, model: &str) -> Json {
    let request = transform_request(input, &json(optional), model, None).unwrap();
    Json::Object(request)
}

#[rstest]
#[case::text_list(
    "text-embedding-005",
    r#"["hello", "world"]"#,
    "{}",
    r#"{"instances": [{"content": "hello"}, {"content": "world"}], "parameters": {}}"#
)]
#[case::text_single(
    "text-embedding-005",
    r#""hello""#,
    "{}",
    r#"{"instances": [{"content": "hello"}], "parameters": {}}"#
)]
#[case::text_empty_list(
    "text-embedding-005",
    "[]",
    "{}",
    r#"{"instances": [], "parameters": {}}"#
)]
#[case::standard_options(
    "text-embedding-005",
    r#"["hello"]"#,
    r#"{"task_type": "CLASSIFICATION", "title": "doc", "auto_truncate": true}"#,
    r#"{"instances": [{"content": "hello", "task_type": "CLASSIFICATION", "title": "doc"}], "parameters": {"task_type": "CLASSIFICATION", "title": "doc", "auto_truncate": true}}"#
)]
#[case::bge_upper(
    "BGE-small-en-v1.5",
    r#"["hello"]"#,
    r#"{"task_type": "RETRIEVAL_DOCUMENT", "title": "doc"}"#,
    r#"{"instances": [{"prompt": "hello", "task_type": "RETRIEVAL_DOCUMENT", "title": "doc"}], "parameters": {"task_type": "RETRIEVAL_DOCUMENT", "title": "doc"}}"#
)]
#[case::bge_prefix(
    "bge/204379420394258432",
    r#""hello""#,
    "{}",
    r#"{"instances": [{"prompt": "hello"}], "parameters": {}}"#
)]
#[case::numeric(
    "1234567890",
    r#"["hello", "world"]"#,
    "{}",
    r#"{"instances": [{"inputs": "hello"}, {"inputs": "world"}], "parameters": {}}"#
)]
#[case::null_options(
    "text-embedding-005",
    r#"["hello"]"#,
    r#"{"task_type": null, "title": null}"#,
    r#"{"instances": [{"content": "hello"}], "parameters": {"task_type": null, "title": null}}"#
)]
fn families_build_their_own_instances(
    #[case] model: &str,
    #[case] input: &str,
    #[case] optional: &str,
    #[case] expected: &str,
) {
    let parsed: EmbeddingInput = serde_json::from_str(input).unwrap();
    assert_eq!(request(&parsed, optional, model), json(expected));
}

#[rstest]
#[case::tuned_drops(
    "1234567890",
    r#"{"instances": [{"inputs": "hello"}], "parameters": {}}"#
)]
#[case::bge_keeps(
    "bge-small",
    r#"{"instances": [{"prompt": "hello"}], "parameters": {"shared_session": "s"}}"#
)]
#[case::standard_keeps(
    "text-embedding-005",
    r#"{"instances": [{"content": "hello"}], "parameters": {"shared_session": "s"}}"#
)]
fn only_the_tuned_branch_drops_shared_session(#[case] model: &str, #[case] expected: &str) {
    let optional = r#"{"shared_session": "s"}"#;
    assert_eq!(request(&text("hello"), optional, model), json(expected));
}

#[test]
fn tuned_parameters_keep_unread_keys() {
    let optional =
        r#"{"shared_session": "s", "task_type": "NOT_A_TASK", "title": 7, "custom": {"a": 1}}"#;
    let built = request(&text("hello"), optional, "42");
    assert_eq!(
        built,
        json(
            r#"{"instances": [{"inputs": "hello"}], "parameters": {"task_type": "NOT_A_TASK", "title": 7, "custom": {"a": 1}}}"#
        )
    );
}

#[rstest]
#[case::standard("text-embedding-005", r#"[{"content": "hello"}]"#)]
#[case::bge("bge-small", r#"[{"prompt": "hello"}]"#)]
#[case::tuned("42", r#"[{"inputs": "hello"}]"#)]
fn labels_attach_to_each_branch(#[case] model: &str, #[case] instances: &str) {
    let optional = r#"{"labels": {"team": "search", "count": 5}, "auto_truncate": false}"#;
    let built = request(&text("hello"), optional, model);
    let expected = format!(
        r#"{{"instances": {instances}, "parameters": {{"auto_truncate": false}}, "labels": {{"team": "search"}}}}"#
    );
    assert_eq!(built, json(&expected));
}

#[test]
fn empty_labels_fall_back_to_requester_metadata() {
    let litellm_params = json(r#"{"metadata": {"requester_metadata": {"team": "search"}}}"#);
    let built = transform_request(
        &text("hello"),
        &json(r#"{"labels": {}}"#),
        "text-embedding-005",
        Some(&litellm_params),
    )
    .unwrap();
    assert_eq!(
        Json::Object(built),
        json(
            r#"{"instances": [{"content": "hello"}], "parameters": {}, "labels": {"team": "search"}}"#
        )
    );
}

#[test]
fn a_non_object_labels_value_is_still_popped() {
    let litellm_params = json(r#"{"litellm_metadata": {"requester_metadata": {"team": "x"}}}"#);
    let built = transform_request(
        &text("hello"),
        &json(r#"{"labels": "ignored"}"#),
        "text-embedding-005",
        Some(&litellm_params),
    )
    .unwrap();
    assert_eq!(
        Json::Object(built),
        json(r#"{"instances": [{"content": "hello"}], "parameters": {}, "labels": {"team": "x"}}"#)
    );
}

#[test]
fn labels_without_text_values_are_omitted() {
    let built = request(
        &text("hello"),
        r#"{"labels": {"count": 1}}"#,
        "text-embedding-005",
    );
    assert_eq!(
        built,
        json(r#"{"instances": [{"content": "hello"}], "parameters": {}}"#)
    );
}

#[rstest]
#[case::unknown_task_type(r#"{"task_type": "classification"}"#, VertexTextEmbeddingError::TaskType { value: String::from("classification") })]
#[case::number_task_type(r#"{"task_type": 4}"#, VertexTextEmbeddingError::NotText { field: "task_type" })]
#[case::number_title(r#"{"title": 4}"#, VertexTextEmbeddingError::NotText { field: "title" })]
fn standard_branches_reject_values_outside_the_declared_fields(
    #[case] optional: &str,
    #[case] expected: VertexTextEmbeddingError,
) {
    let error =
        transform_request(&text("hello"), &json(optional), "text-embedding-005", None).unwrap_err();
    assert_eq!(error, expected);
}

#[test]
fn bge_branches_reject_a_number_title() {
    let error = transform_request(
        &text("hello"),
        &json(r#"{"title": false}"#),
        "bge-small",
        None,
    )
    .unwrap_err();
    assert_eq!(error, VertexTextEmbeddingError::NotText { field: "title" });
}

#[rstest]
#[case::tokens("[1, 2]", VertexTextEmbeddingError::InputNotText)]
#[case::token_lists("[[1, 2]]", VertexTextEmbeddingError::InputNotText)]
fn token_inputs_are_rejected(#[case] input: &str, #[case] expected: VertexTextEmbeddingError) {
    let parsed: EmbeddingInput = serde_json::from_str(input).unwrap();
    let error = transform_request(&parsed, &json("{}"), "text-embedding-005", None).unwrap_err();
    assert_eq!(error, expected);
}

#[test]
fn non_object_optional_params_are_rejected() {
    let error = transform_request(
        &text("hello"),
        &json(r#""nope""#),
        "text-embedding-005",
        None,
    )
    .unwrap_err();
    assert_eq!(error, VertexTextEmbeddingError::OptionalParams);
}
