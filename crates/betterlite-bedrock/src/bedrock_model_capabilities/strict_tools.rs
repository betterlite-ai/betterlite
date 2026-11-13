use betterlite_context::bedrock_model::get_bedrock_base_model;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;

use super::{MODEL_KEY_FIELD, ModelInfoLookup};

const STRICT_TOOLS_FLAG: &str = "bedrock_converse_supports_strict_tools";

#[must_use]
pub async fn converse_supports_strict_tools<Lookup: ModelInfoLookup>(
    model: &str,
    lookup: &Lookup,
    packaged: &ModelCatalog,
) -> bool {
    let base = get_bedrock_base_model(model);
    if !base.starts_with("anthropic") {
        return false;
    }
    for candidate in version_candidates(&base) {
        let Ok(info) = lookup.bedrock_model_info(&candidate).await else {
            continue;
        };
        if let Some(Json::Bool(flag)) = info.fields.get(STRICT_TOOLS_FLAG) {
            return *flag;
        }
        let Some(Json::Text(key)) = info.fields.get(MODEL_KEY_FIELD) else {
            continue;
        };
        let packaged_flag = packaged
            .entries
            .get(key)
            .and_then(|entry| entry.fields.capabilities.vendor.bedrock_strict_tools);
        if let Some(flag) = packaged_flag {
            return flag;
        }
    }
    true
}

fn version_candidates(base: &str) -> Vec<String> {
    let mut candidates = vec![base.to_owned()];
    if let Some(stripped) = strip_version_suffix(base)
        && stripped != base
    {
        candidates.push(stripped);
    }
    candidates
}

// Python's `-v\d+(?::\d+)?$`: the trailing newline stays when `$` matched just before it
fn strip_version_suffix(model: &str) -> Option<String> {
    let body = model.strip_suffix('\n').unwrap_or(model);
    let index = body.rfind("-v")?;
    let tail = body.get(index.checked_add(2)?..)?;
    let (number, revision) = match tail.split_once(':') {
        Some((number, revision)) => (number, Some(revision)),
        None => (tail, None),
    };
    if number.is_empty() || !number.chars().all(|value| value.is_ascii_digit()) {
        return None;
    }
    if let Some(revision) = revision
        && (revision.is_empty() || !revision.chars().all(|value| value.is_ascii_digit()))
    {
        return None;
    }
    let mut stripped = body.get(..index)?.to_owned();
    if model.ends_with('\n') {
        stripped.push('\n');
    }
    Some(stripped)
}
