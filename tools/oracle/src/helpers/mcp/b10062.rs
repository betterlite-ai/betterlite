use betterlite_mcp::redirect_trust::shape::{
    RedirectShape, RedirectUriRejected, is_loopback_redirect_host, native_callbacks,
    parse_redirect_uri, validate_redirect_uri_shape,
};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub(crate) struct Input {
    native_redirect_uris: Option<String>,
    uris: Vec<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let configured = input.native_redirect_uris.as_deref().unwrap_or_default();
        let callbacks = native_callbacks(configured);
        let answers: Vec<serde_json::Value> = input
            .uris
            .iter()
            .map(|uri| answer(uri, &callbacks))
            .collect();
        Ok(json!({"native_callbacks": callbacks, "answers": answers}))
    }
}

fn answer(uri: &str, callbacks: &[String]) -> serde_json::Value {
    let parsed = parse_redirect_uri(uri);
    let loopback = parsed.as_ref().ok().map(is_loopback_redirect_host);
    let shape = parsed.and_then(|parsed_uri| validate_redirect_uri_shape(&parsed_uri, callbacks));
    match shape {
        Ok(RedirectShape::NativeCallback) => {
            json!({"loopback": loopback, "result": {"native_callback": true}})
        }
        Ok(RedirectShape::NeedsTrustPolicy) => {
            json!({"loopback": loopback, "result": {"needs_trust_policy": true}})
        }
        Err(rejected) => {
            let detail = rejected.detail();
            json!({
                "loopback": loopback,
                "refusal": {
                    "status": RedirectUriRejected::STATUS.as_u16(),
                    "detail_error": detail.error,
                },
            })
        }
    }
}
