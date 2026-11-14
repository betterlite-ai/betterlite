use betterlite_context::non_default_params::mapper::DropPolicy;

use crate::image_models::params::{Field, Params};
use crate::marengo_three::is_marengo_3_model;

pub const MARENGO_SHARED_PARAMS: [&str; 11] = [
    "encoding_format",
    "embeddingOption",
    "startSec",
    "input_type",
    "endSec",
    "segmentation",
    "embeddingType",
    "embeddingScope",
    "inferenceId",
    "media_source",
    "media_sources",
];

pub const MARENGO_2_7_ONLY_PARAMS: [&str; 4] =
    ["textTruncate", "lengthSec", "useFixedLengthSec", "minClipSec"];

const PASSTHROUGH_PARAMS: [&str; 13] = [
    "textTruncate",
    "embeddingOption",
    "startSec",
    "lengthSec",
    "useFixedLengthSec",
    "minClipSec",
    "endSec",
    "segmentation",
    "embeddingType",
    "embeddingScope",
    "inferenceId",
    "media_source",
    "media_sources",
];

#[must_use]
pub fn supported_openai_params(model: Option<&str>) -> Vec<&'static str> {
    let mut params = MARENGO_SHARED_PARAMS.to_vec();
    if !is_marengo_3_model(model) {
        params.extend_from_slice(&MARENGO_2_7_ONLY_PARAMS);
    }
    params
}

#[must_use]
pub fn map_openai_params(is_marengo_3: bool, non_default: &Params, mut optional: Params) -> Params {
    for (key, value) in non_default {
        match key.as_str() {
            "encoding_format" => {
                if !is_marengo_3 && value.as_str() == Some("float") {
                    optional.insert("embeddingOption".to_owned(), visual_options());
                }
            }
            "input_type" => {
                optional.insert("inputType".to_owned(), value.clone());
            }
            other if PASSTHROUGH_PARAMS.contains(&other) => {
                optional.insert(key.clone(), value.clone());
            }
            _ => {}
        }
    }
    optional
}

#[must_use]
pub const fn drop_params_enabled(policy: DropPolicy) -> bool {
    policy.litellm_drop_params || matches!(policy.drop_params, Some(true))
}

#[must_use]
pub(super) fn resolved_input_type(inference_params: &Params) -> Field {
    ["inputType", "input_type"]
        .into_iter()
        .find_map(|key| {
            inference_params
                .get(key)
                .filter(|value| value.truthy())
                .cloned()
        })
        .unwrap_or_else(|| Field::Text("text".to_owned()))
}

fn visual_options() -> Field {
    Field::List(vec![
        Field::Text("visual-text".to_owned()),
        Field::Text("visual-image".to_owned()),
    ])
}
