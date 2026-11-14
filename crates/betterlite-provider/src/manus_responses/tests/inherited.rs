use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_openai::response_params::params::supported_openai_params;
use rstest::rstest;

use crate::api::responses_provider_contract::config::ResponsesProviderConfig as _;
use crate::api::responses_provider_contract::records::Params;

use super::support::plain_config;

fn api_params(wire: &str) -> ResponsesApiRequestParams {
    serde_json::from_str(wire).expect("the fixture is a request params record")
}

fn mapped_params(wire: &str) -> Params {
    plain_config()
        .map_params(&api_params(wire), "manus-1.6", false)
        .expect("the params map")
}

#[test]
fn supported_params_match_the_openai_responses_set() {
    let supported = plain_config()
        .supported_params("manus-1.6")
        .expect("the set is static");
    assert_eq!(supported.len(), supported_openai_params().len());
    assert!(supported.contains(&String::from("input")));
    assert!(supported.contains(&String::from("extra_body")));
}

#[test]
fn map_params_drops_input_and_model_and_keeps_the_rest() {
    let mapped = mapped_params(r#"{"input":"hi","model":"manus-1.6","store":false}"#);
    assert!(!mapped.contains_key("input"));
    assert!(!mapped.contains_key("model"));
    assert_eq!(mapped.get("store"), Some(&Json::Bool(false)));
}

#[rstest]
#[case::below_the_minimum(8, 16)]
#[case::just_below(15, 16)]
#[case::at_the_minimum(16, 16)]
#[case::above_the_minimum(17, 17)]
fn map_params_clamps_max_output_tokens_up_to_the_minimum(
    #[case] requested: i64,
    #[case] expected: i64,
) {
    let mapped = mapped_params(&format!(
        r#"{{"input":"hi","model":"manus-1.6","max_output_tokens":{requested}}}"#
    ));
    assert_eq!(
        mapped.get("max_output_tokens"),
        Some(&Json::Number(expected.into()))
    );
}

#[test]
fn map_params_keeps_an_explicit_null_max_output_tokens() {
    let mapped = mapped_params(r#"{"input":"hi","model":"manus-1.6","max_output_tokens":null}"#);
    assert_eq!(mapped.get("max_output_tokens"), Some(&Json::Null));
}

#[test]
fn the_base_transformations_report_their_missing_owner() {
    use crate::api::responses_provider_contract::records::ResponsesLogging;
    use crate::manus_responses::error::ManusResponsesError;

    let config = plain_config();
    let upstream = super::support::response(200, "{}");
    let mut logging = ResponsesLogging::default();
    let chunk: Json = serde_json::from_str(r#"{"type":"vendor.event"}"#).expect("JSON");
    assert!(matches!(
        config.transform_delete_response(&upstream, &mut logging),
        Err(ManusResponsesError::UnsupportedOperation { .. })
    ));
    assert!(matches!(
        config.transform_input_items_response(&upstream, &mut logging),
        Err(ManusResponsesError::UnsupportedOperation { .. })
    ));
    assert!(matches!(
        config.transform_cancel_response(&upstream, &mut logging),
        Err(ManusResponsesError::UnsupportedOperation { .. })
    ));
    assert!(matches!(
        config.transform_compact_response(&upstream, &mut logging),
        Err(ManusResponsesError::UnsupportedOperation { .. })
    ));
    assert!(matches!(
        config.transform_streaming_response("m", &chunk, &mut logging),
        Err(ManusResponsesError::UnsupportedOperation { .. })
    ));
}
