use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Json;

use crate::mantle_responses::error::MantleResponsesError;
use crate::mantle_responses::options::model_map_lookup_name;
use crate::mantle_responses::request::transform_responses_request;

use super::support;

pub(super) fn mantle_model_name() {
    lookup_cases();
    gate_cases();
    outbound_model_cases();
}

fn lookup_cases() {
    let cases = [
        ("openai.gpt-5.5", "gpt-5.5"),
        ("us-east-1/openai.gpt-5.5", "gpt-5.5"),
        ("bedrock_mantle/openai.gpt-oss-120b", "gpt-oss-120b"),
        ("gpt-5.5", "gpt-5.5"),
        ("openai.openai.gpt-5.5", "openai.gpt-5.5"),
        ("a/b/c", "c"),
        ("", ""),
        ("bedrock_mantle/", ""),
        ("openai.", ""),
    ];
    for (model, expected) in cases {
        assert_eq!(model_map_lookup_name(model), expected, "model {model}");
    }
}

fn gate_cases() {
    assert_eq!(
        support::mapped(
            "us-east-1/openai.gpt-5.5-quux",
            r#"{"temperature": 0.5}"#,
            true
        ),
        support::object("{}")
    );
    let error = support::rejected(
        "bedrock_mantle/openai.gpt-5.5-quux",
        r#"{"temperature": 0.5}"#,
        false,
    );
    let MantleResponsesError::Params(inner) = &error else {
        panic!("expected the parent mapping to reject the temperature");
    };
    assert!(format!("{inner}").starts_with(
        "bedrock_mantle/openai.gpt-5.5-quux doesn't support temperature=0.5 while reasoning"
    ));
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(
        support::mapped("bedrock_mantle/", r#"{"temperature": 0.5}"#, false),
        support::object(r#"{"temperature": 0.5}"#)
    );
    assert_eq!(
        support::mapped(
            "us-east-1/openai.gpt-oss-120b",
            r#"{"temperature": 0.5}"#,
            false
        ),
        support::object(r#"{"temperature": 0.5}"#)
    );
}

fn body_model(model: &str) -> Option<String> {
    let input = ResponsesInput::Text("hello".to_owned());
    let params = support::object("{}");
    let body = transform_responses_request(model, &input, &params, None).expect("the body builds");
    body.get("model").and_then(Json::as_str).map(str::to_owned)
}

fn outbound_model_cases() {
    for model in [
        "us-east-1/openai.gpt-5.5",
        "openai.gpt-5.5",
        "",
        "bedrock_mantle/",
    ] {
        assert_eq!(body_model(model).as_deref(), Some(model), "model {model}");
    }
    let input = ResponsesInput::Text("hello".to_owned());
    let params = support::object("{}");
    let body =
        transform_responses_request("openai.gpt-5.5", &input, &params, None).expect("the body");
    assert_eq!(body.get("input"), Some(&Json::Text("hello".to_owned())));
}
