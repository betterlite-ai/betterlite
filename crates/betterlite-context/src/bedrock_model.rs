pub mod model_info;
pub mod regions;
pub mod runtime_arn;
mod tests;

use regex::Regex;

use regions::{AmazonBedrockGlobalConfig, CROSS_REGION_PREFIXES};

const BEDROCK_ROUTING_PREFIXES: [&str; 8] = [
    "bedrock/",
    "chat_completions/",
    "converse/",
    "invoke/",
    "openai/",
    "mantle/",
    "nova-2/",
    "nova/",
];

#[must_use]
pub fn strip_bedrock_routing_prefix(model: &str) -> String {
    let mut stripped = model;
    for prefix in BEDROCK_ROUTING_PREFIXES {
        if let Some(rest) = stripped.strip_prefix(prefix) {
            stripped = rest;
        }
    }
    stripped.to_owned()
}

#[must_use]
pub fn extract_model_from_arn(model: &str) -> String {
    if !model.to_lowercase().contains("arn") {
        return model.to_owned();
    }
    model
        .rsplit('/')
        .next()
        .map_or_else(|| model.to_owned(), str::to_owned)
}

const THROUGHPUT_SUFFIX_PATTERN: &str = r"(:\d+):\d+k$";
const CONTEXT_SUFFIX_PATTERN: &str = r"\[\w+\]$";

#[must_use]
pub fn strip_throughput_suffix(model: &str) -> String {
    let mut result = model.to_owned();
    if let Ok(throughput) = Regex::new(THROUGHPUT_SUFFIX_PATTERN) {
        result = throughput.replace(&result, "$1").into_owned();
    }
    if let Ok(context) = Regex::new(CONTEXT_SUFFIX_PATTERN) {
        result = context.replace(&result, "").into_owned();
    }
    result
}

#[must_use]
pub fn split_bedrock_region_path(model: &str) -> (Option<String>, String) {
    let stripped = strip_bedrock_routing_prefix(model);
    if let Some((region, model_id)) = stripped.split_once('/')
        && AmazonBedrockGlobalConfig
            .get_all_regions()
            .contains(&region)
    {
        return (Some(region.to_owned()), model_id.to_owned());
    }
    (None, stripped)
}

const NOVA_WRAPPER_PREFIXES: [&str; 3] = ["bedrock/converse/", "bedrock/", "converse/"];

#[must_use]
pub fn get_bedrock_base_model(model: &str) -> String {
    let mut wrapped = model;
    for prefix in NOVA_WRAPPER_PREFIXES {
        if let Some(rest) = wrapped.strip_prefix(prefix) {
            wrapped = rest;
            break;
        }
    }
    if wrapped.starts_with("nova-2/") {
        return "amazon.nova-2-custom".to_owned();
    }
    if wrapped.starts_with("nova/") {
        return "amazon.nova-custom".to_owned();
    }

    let routed = strip_bedrock_routing_prefix(model);
    let arn_stripped = extract_model_from_arn(&routed);
    let suffix_stripped = strip_throughput_suffix(&arn_stripped);

    let dot_split = suffix_stripped.split_once('.');
    let potential_region = dot_split.map_or(suffix_stripped.as_str(), |(head, _)| head);
    if CROSS_REGION_PREFIXES.contains(&potential_region) {
        if let Some((_, rest)) = dot_split {
            return rest.to_owned();
        }
        // deviation from the pinned source: a bare prefix with no "." avoids an IndexError (docs/deviations/B2203.yaml)
        return suffix_stripped;
    }

    let slash_split = suffix_stripped.split_once('/');
    let alt_potential_region = slash_split.map_or(suffix_stripped.as_str(), |(head, _)| head);
    if AmazonBedrockGlobalConfig
        .get_all_regions()
        .contains(&alt_potential_region)
        && let Some((_, rest)) = slash_split
    {
        return rest.to_owned();
    }

    suffix_stripped
}
