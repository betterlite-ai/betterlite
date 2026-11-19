use betterlite_openai::passthrough::transformation::types::{PassthroughTarget, RelayedResponse};

use crate::mantle_settings::passthrough::{
    CONVERSE_PROVIDER, logging_non_streaming_response, reply_shape_provider,
};
use crate::passthrough::error::PassthroughFailure;

use super::support;

struct Forwarded {
    model: String,
    provider: String,
    endpoint: String,
    status_code: u16,
    body: Vec<u8>,
    request_data: betterlite_openai::openai_base_config::Object,
}

pub(super) fn mantle_passthrough_reply_shape() {
    assert_eq!(CONVERSE_PROVIDER.to_string(), "bedrock");
    // Only a converse endpoint without invoke takes the native Bedrock reply shape; invoke wins when both occur.
    assert_eq!(
        reply_shape_provider("bedrock_mantle", "converse"),
        "bedrock"
    );
    assert_eq!(
        reply_shape_provider("bedrock_mantle", "model/converse-stream"),
        "bedrock"
    );
    assert_eq!(
        reply_shape_provider("bedrock_mantle", "service/Converse"),
        "bedrock_mantle"
    );
    assert_eq!(
        reply_shape_provider("bedrock_mantle", "invoke"),
        "bedrock_mantle"
    );
    assert_eq!(
        reply_shape_provider("bedrock_mantle", "invoke-with-response-stream"),
        "bedrock_mantle"
    );
    assert_eq!(
        reply_shape_provider("bedrock_mantle", "converse/invoke"),
        "bedrock_mantle"
    );
    assert_eq!(
        reply_shape_provider("bedrock_mantle", "invoke/converse"),
        "bedrock_mantle"
    );
    assert_eq!(
        reply_shape_provider("bedrock_mantle", "chat/completions"),
        "bedrock_mantle"
    );
    assert_eq!(reply_shape_provider("anthropic", "converse"), "bedrock");
    assert_eq!(reply_shape_provider("anthropic", "messages"), "anthropic");
    assert_eq!(reply_shape_provider("bedrock", "converse"), "bedrock");
    // The shaped provider travels on with every other argument untouched.
    let request_data = support::object(r#"{"messages": [], "temperature": 0}"#);
    let body = b"{\"output\": {\"message\": {}}}";
    let response = RelayedResponse {
        status_code: 200,
        body,
    };
    let mut forwarded = Vec::new();
    let result = logging_non_streaming_response(
        PassthroughTarget {
            model: "fixture-model",
            custom_llm_provider: "bedrock_mantle",
            endpoint: "model/converse",
        },
        &response,
        &request_data,
        |target, response, data| {
            forwarded.push(Forwarded {
                model: target.model.to_owned(),
                provider: target.custom_llm_provider.to_owned(),
                endpoint: target.endpoint.to_owned(),
                status_code: response.status_code,
                body: response.body.to_vec(),
                request_data: data.clone(),
            });
            Ok(Some("logged-reply"))
        },
    )
    .expect("the reply is logged");
    assert_eq!(result, Some("logged-reply"));
    assert_eq!(forwarded.len(), 1);
    let seen = forwarded.first().expect("the forward ran once");
    assert_eq!(seen.model, "fixture-model");
    assert_eq!(seen.provider, "bedrock");
    assert_eq!(seen.endpoint, "model/converse");
    assert_eq!(seen.status_code, 200);
    assert_eq!(seen.body.as_slice(), body.as_slice());
    assert_eq!(&seen.request_data, &request_data);
    // An invoke endpoint keeps the calling provider.
    let mut invoke_provider = String::new();
    let logged = logging_non_streaming_response(
        PassthroughTarget {
            model: "fixture-model",
            custom_llm_provider: "bedrock_mantle",
            endpoint: "model/invoke",
        },
        &response,
        &request_data,
        |target, _response, _data| -> Result<Option<&'static str>, PassthroughFailure> {
            invoke_provider = target.custom_llm_provider.to_owned();
            Ok(None)
        },
    )
    .expect("the invoke reply is forwarded");
    assert_eq!(logged, None);
    assert_eq!(invoke_provider, "bedrock_mantle");
    // A failed forward comes back unchanged, after seeing the shaped provider.
    let mut shaped = String::new();
    let failure = logging_non_streaming_response(
        PassthroughTarget {
            model: "fixture-model",
            custom_llm_provider: "bedrock_mantle",
            endpoint: "converse",
        },
        &response,
        &request_data,
        |target, _response, _data| -> Result<Option<&'static str>, PassthroughFailure> {
            shaped = target.custom_llm_provider.to_owned();
            Err(PassthroughFailure::Field {
                field: "fixture-field",
            })
        },
    )
    .expect_err("the forward failure returns");
    assert!(matches!(
        failure,
        PassthroughFailure::Field {
            field: "fixture-field"
        }
    ));
    assert_eq!(
        failure.to_string(),
        "the field fixture-field is not a string"
    );
    assert_eq!(shaped, "bedrock");
}
