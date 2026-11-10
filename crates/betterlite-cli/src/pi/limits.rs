use std::collections::BTreeMap;

use num_traits::ToPrimitive as _;
use secrecy::SecretString;
use serde::Deserialize;

use super::{FETCH_TIMEOUT, ModelLimits, ProxyGet};

#[derive(Deserialize)]
struct ModelGroupList {
    data: Vec<ModelGroup>,
}

#[derive(Deserialize)]
struct ModelGroup {
    #[serde(rename = "model_group")]
    name: String,
    max_input_tokens: Option<f64>,
    max_output_tokens: Option<f64>,
}

#[must_use]
pub fn fetch_model_limits(
    base_url: &str,
    api_key: &SecretString,
    get: &dyn ProxyGet,
) -> BTreeMap<String, ModelLimits> {
    let url = format!("{}/model_group/info", base_url.trim_end_matches('/'));
    let sent_headers = super::request_headers(api_key, &[]);
    let Ok(response) = get.get(&url, &sent_headers, FETCH_TIMEOUT) else {
        return BTreeMap::new();
    };
    if response.status != 200 {
        return BTreeMap::new();
    }
    let Ok(listing) = serde_json::from_slice::<ModelGroupList>(&response.body) else {
        return BTreeMap::new();
    };
    // Python's int(value) truncates toward zero; a limit outside i64 has no JSON number to write
    listing
        .data
        .into_iter()
        .map(|group| {
            (
                group.name,
                ModelLimits {
                    context_window: group.max_input_tokens.and_then(|tokens| tokens.to_i64()),
                    max_tokens: group.max_output_tokens.and_then(|tokens| tokens.to_i64()),
                },
            )
        })
        .collect()
}
