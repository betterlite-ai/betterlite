use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest,
};
use betterlite_provider::vllm::passthrough::VllmPassthroughConfig;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PassInput {
    pub(super) calls: Vec<PassCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PassCall {
    endpoint: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    query: Option<Object>,
    #[serde(default)]
    request_data: Object,
}

pub(super) fn passthrough_value(call: &PassCall) -> Value {
    let config = VllmPassthroughConfig {
        environment: &call.env,
    };
    let litellm_params = Object::new();
    let request = PassthroughUrlRequest {
        api_base: call.api_base.as_deref(),
        api_key: None,
        target: PassthroughTarget {
            model: "m",
            custom_llm_provider: "vllm",
            endpoint: &call.endpoint,
        },
        request_query_params: call.query.as_ref(),
        litellm_params: &litellm_params,
    };
    let result = match config.get_complete_url(&request) {
        Ok((url, base)) => json!({"ok": true, "url": url.as_str(), "base": base}),
        Err(_) => json!({"ok": false, "status": null}),
    };
    json!({
        "streaming": config.is_streaming_request(&call.endpoint, &call.request_data),
        "result": result,
    })
}
