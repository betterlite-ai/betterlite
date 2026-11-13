use std::collections::BTreeSet;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;

use crate::api::fal::gpt_image::MODEL_PREFIX;

#[must_use]
pub fn supported_gpt_image_qualities(model: &str, catalog: &ModelCatalog) -> BTreeSet<String> {
    let endpoint = model.strip_prefix("fal_ai/").unwrap_or(model);
    let qualified = if endpoint.starts_with(MODEL_PREFIX) {
        endpoint.to_owned()
    } else {
        format!("{MODEL_PREFIX}{endpoint}")
    };
    let mut qualities = BTreeSet::new();
    for key in catalog.entries.keys() {
        let mut parts = key.split('/');
        let Some(provider) = parts.next() else {
            continue;
        };
        let Some(quality) = parts.next() else {
            continue;
        };
        let Some(dimension) = parts.next() else {
            continue;
        };
        let suffix: Vec<&str> = parts.collect();
        if provider != "fal_ai" || suffix.is_empty() || !dimension.contains("-x-") {
            continue;
        }
        if suffix.join("/") == qualified {
            qualities.insert(quality.to_owned());
        }
    }
    if !qualities.is_empty() {
        qualities.insert("auto".to_owned());
    }
    qualities
}

#[must_use]
pub fn map_gpt_image_quality(quality: &Json, model: &str, catalog: &ModelCatalog) -> Json {
    let Json::Text(text) = quality else {
        return quality.clone();
    };
    let normalized = normalize_quality(text);
    let supported = supported_gpt_image_qualities(model, catalog);
    if supported.is_empty() || supported.contains(normalized) {
        return Json::Text(normalized.to_owned());
    }
    Json::Text("auto".to_owned())
}

fn normalize_quality(quality: &str) -> &str {
    match quality {
        "hd" => "high",
        "standard" => "medium",
        _ => quality,
    }
}
