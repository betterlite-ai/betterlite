#![cfg(test)]

use std::fs;

use betterlite_openai::embedding::{EmbeddingInput, EmbeddingVector};
use betterlite_prompts::arguments::json::Json;
use http::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::oci_common::OciServingMode;
use crate::oci_common::credentials::{DEFAULT_OCI_REGION, OciCredentials};
use crate::oci_common::environment::OciEnvironment;

use super::response::decode_embed_response;
use super::{
    OCI_EMBED_BATCH_LIMIT, OciEmbedParams, OciEmbedRequest, SUPPORTED_OPENAI_PARAMS,
    get_complete_url, map_openai_params, transform_embedding_request, validate_environment,
};

const COMPARTMENT: &str = "ocid1.compartment.oc1..aaaa";

fn credentials() -> OciCredentials {
    OciCredentials {
        oci_region: DEFAULT_OCI_REGION.to_owned(),
        oci_user: Some("ocid1.user.oc1..bbbb".to_owned()),
        oci_fingerprint: Some("aa:bb:cc".to_owned()),
        oci_tenancy: Some("ocid1.tenancy.oc1..cccc".to_owned()),
        oci_key: Some(SecretString::from("-----BEGIN PRIVATE KEY-----")),
        oci_key_file: None,
        oci_compartment_id: Some(COMPARTMENT.to_owned()),
    }
}

fn options() -> OciEmbedParams {
    OciEmbedParams {
        dimensions: None,
        output_dimensions: None,
        input_type: None,
        truncate: None,
        oci_serving_mode: None,
        oci_endpoint_id: None,
    }
}

fn request_with(options: &OciEmbedParams) -> OciEmbedRequest {
    transform_embedding_request(
        "cohere.embed-v4.0",
        &EmbeddingInput::Text("hello".to_owned()),
        options,
        &credentials(),
    )
    .unwrap()
}

fn body_json(options: &OciEmbedParams) -> Json {
    let encoded = serde_json::to_string(&request_with(options)).unwrap();
    serde_json::from_str(&encoded).unwrap()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

#[test]
fn supported_openai_params_hold_only_dimensions() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["dimensions"]);
}

#[test]
fn map_openai_params_writes_dimensions_to_output_dimensions() {
    let mut non_default = options();
    non_default.dimensions = Some(1024);
    let mapped = map_openai_params(&non_default, options());
    assert_eq!(mapped.output_dimensions, Some(1024));
}

#[test]
fn map_openai_params_keeps_the_other_optional_params() {
    let mut non_default = options();
    non_default.dimensions = Some(256);
    let mut optional = options();
    optional.oci_serving_mode = Some("DEDICATED".to_owned());
    optional.input_type = Some("search_query".to_owned());
    let mapped = map_openai_params(&non_default, optional);
    assert_eq!(mapped.output_dimensions, Some(256));
    assert_eq!(mapped.oci_serving_mode.as_deref(), Some("DEDICATED"));
    assert_eq!(mapped.input_type.as_deref(), Some("search_query"));
}

#[test]
fn map_openai_params_leaves_optional_params_alone_without_dimensions() {
    let mut non_default = options();
    non_default.input_type = Some("clustering".to_owned());
    let mut optional = options();
    optional.output_dimensions = Some(512);
    let mapped = map_openai_params(&non_default, optional);
    assert_eq!(mapped.output_dimensions, Some(512));
    assert_eq!(mapped.input_type, None);
}

#[test]
fn map_openai_params_prefers_dimensions_over_an_existing_output_dimensions() {
    let mut non_default = options();
    non_default.dimensions = Some(1024);
    let mut optional = options();
    optional.output_dimensions = Some(512);
    let mapped = map_openai_params(&non_default, optional);
    assert_eq!(mapped.output_dimensions, Some(1024));
}

#[test]
fn mapped_params_serialize_with_the_oci_output_dimensions_name() {
    let mut non_default = options();
    non_default.dimensions = Some(1024);
    let encoded = serde_json::to_string(&map_openai_params(&non_default, options())).unwrap();
    assert_eq!(encoded, r#"{"outputDimensions":1024}"#);
}

#[test]
fn params_deserialize_the_optional_param_names() {
    let params: OciEmbedParams = serde_json::from_str(
        r#"{"dimensions":8,"outputDimensions":512,"input_type":"search_query","truncate":"START","oci_serving_mode":"DEDICATED","oci_endpoint_id":"endpoint"}"#,
    )
    .unwrap();
    assert_eq!(params.dimensions, Some(8));
    assert_eq!(params.output_dimensions, Some(512));
    assert_eq!(params.input_type.as_deref(), Some("search_query"));
    assert_eq!(params.truncate.as_deref(), Some("START"));
    assert_eq!(params.oci_serving_mode.as_deref(), Some("DEDICATED"));
    assert_eq!(params.oci_endpoint_id.as_deref(), Some("endpoint"));
}

#[test]
fn params_ignore_an_unknown_optional_param() {
    let params: OciEmbedParams = serde_json::from_str(r#"{"user":"ignored"}"#).unwrap();
    assert_eq!(params, options());
}

#[test]
fn transform_request_builds_the_on_demand_body() {
    let body = request_with(&options());
    assert_eq!(body.compartment_id, COMPARTMENT);
    assert_eq!(
        body.serving_mode,
        OciServingMode::OnDemand {
            model_id: "cohere.embed-v4.0".to_owned()
        }
    );
    assert_eq!(body.inputs, ["hello"]);
    assert_eq!(body.input_type, None);
    assert_eq!(body.truncate, "END");
    assert_eq!(body.output_dimensions, None);
}

#[test]
fn transform_request_skips_the_none_fields_on_the_wire() {
    assert_eq!(
        body_json(&options()),
        json(
            r#"{"compartmentId":"ocid1.compartment.oc1..aaaa","servingMode":{"servingType":"ON_DEMAND","modelId":"cohere.embed-v4.0"},"inputs":["hello"],"truncate":"END"}"#
        )
    );
}

#[test]
fn transform_request_sends_input_type_and_dimensions_when_set() {
    let mut options = options();
    options.input_type = Some("SEARCH_DOCUMENT".to_owned());
    options.output_dimensions = Some(1024);
    assert_eq!(
        body_json(&options),
        json(
            r#"{"compartmentId":"ocid1.compartment.oc1..aaaa","servingMode":{"servingType":"ON_DEMAND","modelId":"cohere.embed-v4.0"},"inputs":["hello"],"inputType":"SEARCH_DOCUMENT","truncate":"END","outputDimensions":1024}"#
        )
    );
}

#[test]
fn transform_request_uses_the_dedicated_endpoint_id() {
    let mut options = options();
    options.oci_serving_mode = Some("DEDICATED".to_owned());
    options.oci_endpoint_id = Some("ocid1.generativeaiendpoint.oc1..dddd".to_owned());
    assert_eq!(
        body_json(&options),
        json(
            r#"{"compartmentId":"ocid1.compartment.oc1..aaaa","servingMode":{"servingType":"DEDICATED","endpointId":"ocid1.generativeaiendpoint.oc1..dddd"},"inputs":["hello"],"truncate":"END"}"#
        )
    );
}

#[test]
fn transform_request_falls_back_to_the_model_for_a_dedicated_endpoint() {
    let mut options = options();
    options.oci_serving_mode = Some("dedicated".to_owned());
    assert_eq!(
        body_json(&options),
        json(
            r#"{"compartmentId":"ocid1.compartment.oc1..aaaa","servingMode":{"servingType":"DEDICATED","endpointId":"cohere.embed-v4.0"},"inputs":["hello"],"truncate":"END"}"#
        )
    );
}

#[rstest]
#[case::lowercase("on_demand", "ON_DEMAND")]
#[case::mixed_case("On_Demand", "ON_DEMAND")]
#[case::upper_case("DEDICATED", "DEDICATED")]
fn transform_request_reads_the_serving_mode_case_insensitively(
    #[case] mode: &str,
    #[case] expected: &str,
) {
    let mut options = options();
    options.oci_serving_mode = Some(mode.to_owned());
    let serving_type = match request_with(&options).serving_mode {
        OciServingMode::OnDemand { .. } => "ON_DEMAND",
        OciServingMode::Dedicated { .. } => "DEDICATED",
    };
    assert_eq!(serving_type, expected);
}

#[rstest]
#[case::unknown("AUTO")]
#[case::empty("")]
fn transform_request_rejects_an_unknown_serving_mode(#[case] mode: &str) {
    let mut options = options();
    options.oci_serving_mode = Some(mode.to_owned());
    let error = transform_embedding_request(
        "cohere.embed-v4.0",
        &EmbeddingInput::Text("hello".to_owned()),
        &options,
        &credentials(),
    )
    .unwrap_err();
    assert_eq!(error.status, 400);
}

#[rstest]
#[case::search_document("search_document", "SEARCH_DOCUMENT")]
#[case::search_query("SEARCH_QUERY", "SEARCH_QUERY")]
#[case::classification("Classification", "CLASSIFICATION")]
#[case::clustering("clustering", "CLUSTERING")]
#[case::unknown("image", "IMAGE")]
#[case::unknown_with_separator("search-image", "SEARCH-IMAGE")]
#[case::empty("", "")]
fn transform_request_upper_cases_the_input_type(#[case] input_type: &str, #[case] expected: &str) {
    let mut options = options();
    options.input_type = Some(input_type.to_owned());
    let body = request_with(&options);
    assert_eq!(body.input_type.as_deref(), Some(expected));
}

#[rstest]
#[case::default_provided("NONE")]
#[case::start("START")]
#[case::end("END")]
fn transform_request_keeps_an_explicit_truncate(#[case] truncate: &str) {
    let mut options = options();
    options.truncate = Some(truncate.to_owned());
    assert_eq!(request_with(&options).truncate, truncate);
}

#[test]
fn transform_request_keeps_an_empty_truncate_as_given() {
    let mut options = options();
    options.truncate = Some(String::new());
    assert_eq!(request_with(&options).truncate, "");
}

#[test]
fn transform_request_defaults_a_missing_truncate_to_end() {
    assert_eq!(request_with(&options()).truncate, "END");
}

#[test]
fn transform_request_wraps_a_single_string_input() {
    let body = transform_embedding_request(
        "cohere.embed-v4.0",
        &EmbeddingInput::Text("one text".to_owned()),
        &options(),
        &credentials(),
    )
    .unwrap();
    assert_eq!(body.inputs, ["one text"]);
}

#[test]
fn transform_request_keeps_a_text_list_in_order() {
    let body = transform_embedding_request(
        "cohere.embed-v4.0",
        &EmbeddingInput::Texts(vec!["first".to_owned(), "second".to_owned()]),
        &options(),
        &credentials(),
    )
    .unwrap();
    assert_eq!(body.inputs, ["first", "second"]);
}

#[test]
fn transform_request_stringifies_token_ids() {
    let body = transform_embedding_request(
        "cohere.embed-v4.0",
        &EmbeddingInput::Tokens(vec![1, 42]),
        &options(),
        &credentials(),
    )
    .unwrap();
    assert_eq!(body.inputs, ["1", "42"]);
}

#[test]
fn transform_request_rejects_token_arrays() {
    let error = transform_embedding_request(
        "cohere.embed-v4.0",
        &EmbeddingInput::TokenLists(vec![vec![1, 2]]),
        &options(),
        &credentials(),
    )
    .unwrap_err();
    assert_eq!(error.status, 400);
    assert!(error.message.contains("token-array"));
}

#[test]
fn transform_request_accepts_the_batch_limit() {
    let texts = (0..OCI_EMBED_BATCH_LIMIT)
        .map(|index| format!("text {index}"))
        .collect();
    let body = transform_embedding_request(
        "cohere.embed-v4.0",
        &EmbeddingInput::Texts(texts),
        &options(),
        &credentials(),
    )
    .unwrap();
    assert_eq!(body.inputs.len(), OCI_EMBED_BATCH_LIMIT);
}

#[test]
fn transform_request_rejects_one_input_past_the_batch_limit() {
    let texts = (0..=OCI_EMBED_BATCH_LIMIT)
        .map(|index| format!("text {index}"))
        .collect();
    let error = transform_embedding_request(
        "cohere.embed-v4.0",
        &EmbeddingInput::Texts(texts),
        &options(),
        &credentials(),
    )
    .unwrap_err();
    assert_eq!(error.status, 400);
    assert!(error.message.contains("got 97"));
}

#[test]
fn transform_request_requires_the_compartment_id() {
    let mut credentials = credentials();
    credentials.oci_compartment_id = None;
    let error = transform_embedding_request(
        "cohere.embed-v4.0",
        &EmbeddingInput::Text("hello".to_owned()),
        &options(),
        &credentials,
    )
    .unwrap_err();
    assert_eq!(error.status, 400);
    assert!(error.message.contains("oci_compartment_id"));
}

#[test]
fn transform_request_treats_an_empty_compartment_id_as_missing() {
    let mut credentials = credentials();
    credentials.oci_compartment_id = Some(String::new());
    let error = transform_embedding_request(
        "cohere.embed-v4.0",
        &EmbeddingInput::Text("hello".to_owned()),
        &options(),
        &credentials,
    )
    .unwrap_err();
    assert_eq!(error.status, 400);
}

#[test]
fn validate_environment_adds_the_oci_headers() {
    let mut headers = HeaderMap::new();
    validate_environment(&mut headers, &credentials(), &OciEnvironment::default()).unwrap();
    assert_eq!(headers.get("content-type").unwrap(), "application/json");
    assert_eq!(headers.get("user-agent").unwrap(), "litellm/1.105.1");
}

#[test]
fn validate_environment_accepts_a_key_file_as_the_key_source() {
    let mut credentials = credentials();
    credentials.oci_key = None;
    credentials.oci_key_file = Some("/keys/oci.pem".to_owned());
    let mut headers = HeaderMap::new();
    validate_environment(&mut headers, &credentials, &OciEnvironment::default()).unwrap();
    assert!(headers.contains_key("user-agent"));
}

#[rstest]
#[case::missing_user("oci_user", None)]
#[case::missing_fingerprint("oci_fingerprint", None)]
#[case::missing_tenancy("oci_tenancy", None)]
#[case::missing_compartment("oci_compartment_id", None)]
#[case::missing_key("oci_key", None)]
#[case::empty_user("oci_user", Some(""))]
#[case::empty_fingerprint("oci_fingerprint", Some(""))]
#[case::empty_tenancy("oci_tenancy", Some(""))]
#[case::empty_compartment("oci_compartment_id", Some(""))]
#[case::empty_key("oci_key", Some(""))]
fn validate_environment_rejects_a_missing_or_empty_credential(
    #[case] name: &str,
    #[case] value: Option<&str>,
) {
    let mut credentials = credentials();
    set_credential(&mut credentials, name, value);
    let mut headers = HeaderMap::new();
    let error =
        validate_environment(&mut headers, &credentials, &OciEnvironment::default()).unwrap_err();
    assert_eq!(error.status, 401);
    assert!(headers.is_empty());
}

fn set_credential(credentials: &mut OciCredentials, name: &str, value: Option<&str>) {
    match name {
        "oci_user" => credentials.oci_user = value.map(str::to_owned),
        "oci_fingerprint" => credentials.oci_fingerprint = value.map(str::to_owned),
        "oci_tenancy" => credentials.oci_tenancy = value.map(str::to_owned),
        "oci_compartment_id" => credentials.oci_compartment_id = value.map(str::to_owned),
        "oci_key" => credentials.oci_key = value.map(SecretString::from),
        other => panic!("unknown credential {other}"),
    }
}

#[test]
fn embed_text_url_appends_the_action_to_the_region_base() {
    let url = get_complete_url(&credentials(), None).unwrap();
    assert_eq!(
        url,
        "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com/20231130/actions/embedText"
    );
}

#[test]
fn embed_text_url_strips_a_previous_action_path_from_the_api_base() {
    let url =
        get_complete_url(&credentials(), Some("https://host/20231130/actions/chat/")).unwrap();
    assert_eq!(url, "https://host/20231130/actions/embedText");
}

#[test]
fn embed_text_url_trims_a_trailing_slash_from_the_api_base() {
    let url = get_complete_url(&credentials(), Some("https://host/base///")).unwrap();
    assert_eq!(url, "https://host/base/20231130/actions/embedText");
}

#[test]
fn embed_text_url_rejects_an_invalid_region() {
    let mut credentials = credentials();
    credentials.oci_region = "US_ASHBURN".to_owned();
    let error = get_complete_url(&credentials, None).unwrap_err();
    assert_eq!(error.status, 400);
}

#[test]
fn decode_response_returns_the_raw_body_for_a_non_200_status() {
    let error = decode_embed_response(503, b"upstream is down").unwrap_err();
    assert_eq!(error.status, 503);
    assert_eq!(error.message, "upstream is down");
}

#[test]
fn decode_response_checks_the_status_before_the_body() {
    let error = decode_embed_response(400, b"{\"embeddings\": []}").unwrap_err();
    assert_eq!(error.status, 400);
}

#[test]
fn decode_response_keeps_the_http_status_for_invalid_json() {
    let error = decode_embed_response(200, b"{not json").unwrap_err();
    assert_eq!(error.status, 200);
    assert!(
        error
            .message
            .starts_with("Failed to parse OCI embed response as JSON:")
    );
}

#[test]
fn decode_response_keeps_the_http_status_for_a_non_utf8_body() {
    let error = decode_embed_response(200, &[0xff_u8, 0xfe]).unwrap_err();
    assert_eq!(error.status, 200);
}

#[rstest]
#[case::missing_embeddings(r#"{"modelId":"m","modelVersion":"v"}"#)]
#[case::missing_model_id(r#"{"embeddings":[[0.5]],"modelVersion":"v"}"#)]
#[case::missing_model_version(r#"{"embeddings":[[0.5]],"modelId":"m"}"#)]
#[case::wrong_embedding_type(r#"{"embeddings":"x","modelId":"m","modelVersion":"v"}"#)]
#[case::non_object_body("[]")]
#[case::full_length_array(r#"[null,[[0.5]],"m","v"]"#)]
#[case::usage_array(r#"{"embeddings":[[0.5]],"modelId":"m","modelVersion":"v","usage":[4,7]}"#)]
#[case::null_body("null")]
#[case::negative_token_count(
    r#"{"embeddings":[[0.5]],"modelId":"m","modelVersion":"v","inputTextTokenCounts":[-1]}"#
)]
fn decode_response_rejects_a_body_that_does_not_fit(#[case] body: &str) {
    let error = decode_embed_response(200, body.as_bytes()).unwrap_err();
    assert_eq!(error.status, 500);
    assert!(
        error
            .message
            .starts_with("OCI embed response does not match expected schema:")
    );
}

#[test]
fn decode_response_maps_rows_with_their_index() {
    let response = decode_embed_response(
        200,
        br#"{"id":"r1","embeddings":[[0.5,-1.25],[2.0]],"modelId":"cohere.embed-v4.0","modelVersion":"1.2"}"#,
    )
    .unwrap();
    assert_eq!(response.object, "list");
    assert_eq!(response.model, "cohere.embed-v4.0");
    assert_eq!(response.data.len(), 2);
    assert_eq!(response.data[0].object, "embedding");
    assert_eq!(response.data[0].index, 0);
    assert_eq!(
        response.data[0].embedding,
        EmbeddingVector::Floats(vec![
            serde_json::Number::from_f64(0.5).unwrap(),
            serde_json::Number::from_f64(-1.25).unwrap()
        ])
    );
    assert_eq!(response.data[1].index, 1);
}

#[test]
fn decode_response_keeps_an_empty_embedding_list() {
    let response = decode_embed_response(
        200,
        br#"{"embeddings":[],"modelId":"m","modelVersion":"v"}"#,
    )
    .unwrap();
    assert!(response.data.is_empty());
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.total_tokens, 0);
}

#[test]
fn decode_response_sums_the_input_text_token_counts() {
    let response = decode_embed_response(
        200,
        br#"{"embeddings":[[0.5]],"modelId":"m","modelVersion":"v","inputTextTokenCounts":[1,2,3],"usage":{"promptTokens":9,"totalTokens":10}}"#,
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 6);
    assert_eq!(response.usage.total_tokens, 6);
    assert_eq!(response.usage.completion_tokens, 0);
}

#[test]
fn decode_response_lets_an_empty_token_count_list_win_over_usage() {
    let response = decode_embed_response(
        200,
        br#"{"embeddings":[[0.5]],"modelId":"m","modelVersion":"v","inputTextTokenCounts":[],"usage":{"promptTokens":9,"totalTokens":10}}"#,
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.total_tokens, 0);
}

#[test]
fn decode_response_uses_the_usage_object_when_the_counts_are_absent() {
    let response = decode_embed_response(
        200,
        br#"{"embeddings":[[0.5]],"modelId":"m","modelVersion":"v","usage":{"promptTokens":4,"totalTokens":7}}"#,
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 4);
    assert_eq!(response.usage.total_tokens, 7);
}

#[test]
fn decode_response_defaults_usage_to_zero() {
    let response = decode_embed_response(
        200,
        br#"{"embeddings":[[0.5]],"modelId":"m","modelVersion":"v"}"#,
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.total_tokens, 0);
}

#[test]
fn decode_response_rejects_an_usage_object_without_total_tokens() {
    let error = decode_embed_response(
        200,
        br#"{"embeddings":[[0.5]],"modelId":"m","modelVersion":"v","usage":{"promptTokens":4}}"#,
    )
    .unwrap_err();
    assert_eq!(error.status, 500);
}

#[test]
fn decode_response_runs_the_token_count_overflow_check() {
    let error = decode_embed_response(
        200,
        br#"{"embeddings":[[0.5]],"modelId":"m","modelVersion":"v","inputTextTokenCounts":[4294967295,1]}"#,
    )
    .unwrap_err();
    assert_eq!(error.status, 500);
    assert!(error.message.contains("overflow"));
}

#[test]
fn decode_response_ignores_an_unknown_top_level_field() {
    let response = decode_embed_response(
        200,
        br#"{"embeddings":[[0.5]],"modelId":"m","modelVersion":"v","extra":"kept out"}"#,
    )
    .unwrap();
    assert_eq!(response.model, "m");
}

#[test]
fn decode_response_accepts_an_integer_vector_component() {
    let response = decode_embed_response(
        200,
        br#"{"embeddings":[[1]],"modelId":"m","modelVersion":"v"}"#,
    )
    .unwrap();
    assert_eq!(
        response.data[0].embedding,
        EmbeddingVector::Floats(vec![serde_json::Number::from(1_u32)])
    );
}

#[test]
fn replays_oci_embed_response_corpus() {
    let seen = fs::read_dir("fuzz/corpus/oci_embed_response")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).unwrap();
            let _decoded = decode_embed_response(200, &bytes);
            let _rejected = decode_embed_response(500, &bytes);
        })
        .count();
    assert!(seen > 0_usize);
}
