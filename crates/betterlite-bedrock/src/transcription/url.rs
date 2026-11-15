use std::collections::BTreeMap;

use super::params::{Params, text_param};

const DEFAULT_REGION: &str = "us-west-2";

fn is_bedrock_region(value: &str) -> bool {
    value.len() > 3
        && value.contains('-')
        && value
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '-')
}

#[must_use]
pub fn bedrock_model_id_and_region(model: &str) -> (&str, Option<&str>) {
    let mut stripped = model;
    for prefix in ["bedrock/converse/", "bedrock/", "converse/"] {
        if let Some(value) = stripped.strip_prefix(prefix) {
            stripped = value;
            break;
        }
    }
    let mut region = None;
    if let Some((candidate, remainder)) = stripped.split_once('/')
        && is_bedrock_region(candidate)
    {
        region = Some(candidate);
        stripped = remainder;
    }
    for prefix in ["nova-2/", "nova/"] {
        if let Some(value) = stripped.strip_prefix(prefix) {
            stripped = value;
            break;
        }
    }
    if region.is_none() {
        region = stripped
            .strip_prefix("arn:")
            .and_then(|arn| arn.split(':').nth(2))
            .filter(|field| !field.is_empty());
    }
    (stripped, region)
}

#[must_use]
pub fn resolve_region(
    model_region: Option<&str>,
    params: &Params,
    env: &BTreeMap<String, String>,
) -> String {
    if let Some(region) = text_param(params, "aws_region_name") {
        return region.to_owned();
    }
    if let Some(region) = model_region {
        return region.to_owned();
    }
    if let Some(region) = env.get("AWS_REGION_NAME").or_else(|| env.get("AWS_REGION")) {
        return region.to_owned();
    }
    DEFAULT_REGION.to_owned()
}

fn runtime_endpoint(
    api_base: Option<&str>,
    model_region: Option<&str>,
    params: &Params,
    env: &BTreeMap<String, String>,
) -> String {
    let configured = text_param(params, "aws_bedrock_runtime_endpoint")
        .or(api_base)
        .map(str::trim)
        .filter(|endpoint| !endpoint.is_empty());
    if let Some(endpoint) = configured {
        return endpoint.to_owned();
    }
    let region = resolve_region(model_region, params, env);
    format!("https://bedrock-runtime.{region}.amazonaws.com")
}

#[must_use]
pub fn complete_url(
    api_base: Option<&str>,
    model: &str,
    params: &Params,
    env: &BTreeMap<String, String>,
) -> String {
    let (model_id, model_region) = bedrock_model_id_and_region(model);
    let endpoint = runtime_endpoint(api_base, model_region, params, env);
    format!(
        "{}/model/{model_id}/converse",
        endpoint.trim_end_matches('/')
    )
}
