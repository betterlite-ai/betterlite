use secrecy::SecretString;
use serde_json::{Value, json};

use super::{Case, UrlCase};
use betterlite_openai::nvidia_nim_passthrough::{
    NvidiaNimPassthroughConfig, validate_environment as validate_passthrough_environment,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest,
};

const PROVIDER: &str = "nvidia_nim";

fn metadata(model_group: Option<&str>) -> Object {
    let mut params = Object::new();
    if let Some(group) = model_group {
        params.insert("litellm_metadata".to_owned(), json!({"model_group": group}));
    }
    params
}

pub(crate) fn urls(cases: &[UrlCase]) -> Value {
    let urls: Vec<Value> = cases
        .iter()
        .map(|case| {
            let config = NvidiaNimPassthroughConfig {
                api_base_env: case.env_base.clone(),
                api_key_env: None,
            };
            let litellm_params = metadata(case.model_group.as_deref());
            let target = PassthroughTarget {
                model: &case.model,
                custom_llm_provider: PROVIDER,
                endpoint: &case.endpoint,
            };
            let request = PassthroughUrlRequest {
                api_base: case.api_base.as_deref(),
                api_key: None,
                target,
                request_query_params: None,
                litellm_params: &litellm_params,
            };
            match config.get_complete_url(&request) {
                Ok((url, root)) => json!({"ok": true, "url": url.as_str(), "root": root}),
                Err(_) => json!({"ok": false}),
            }
        })
        .collect();
    json!({"urls": urls})
}

pub(crate) fn credentials(cases: &[Case]) -> Value {
    let config = NvidiaNimPassthroughConfig::default();
    let headers: Vec<Value> = cases
        .iter()
        .map(|case| {
            let api_key = case.api_key.as_ref().map(SecretString::from);
            json!(validate_passthrough_environment(
                case.headers.clone(),
                api_key.as_ref()
            ))
        })
        .collect();
    let streaming: Vec<Value> = cases
        .iter()
        .map(|case| {
            let request_data = json!({"stream": case.stream.clone().unwrap_or(Value::Null)});
            json!(config.is_streaming_request(&case.endpoint, &super::object(request_data)))
        })
        .collect();
    json!({"headers": headers, "streaming": streaming})
}
