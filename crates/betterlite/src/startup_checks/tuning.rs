use std::collections::BTreeSet;

use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use betterlite_models::key_digest::hashing::hash_token;
use betterlite_models::model_registration::JsonObject;
use betterlite_server::router_dependencies::config_write::validate_config_write;
use indexmap::IndexMap;

pub const AUTO_ROUTER_COMPLEXITY_PREFIX: &str = "auto_router/complexity_router";

// router_utils/auto_router_tuning_baseline.py's identity-to-fingerprint mapping, in deployment order
pub type TuningBaselines = IndexMap<String, String>;

// the fields of a deployment the tuning baseline gate reads; other keys stay with the router
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deployment {
    pub model_name: Option<String>,
    pub model: Option<String>,
    pub tags: Vec<String>,
    pub model_info_id: Option<String>,
    pub db_model: bool,
    pub complexity_router_config: Option<Json>,
}

impl Deployment {
    #[must_use]
    pub fn from_entry(entry: &Json) -> Self {
        let empty = JsonObject::new();
        let fields = match entry {
            Json::Object(fields) => fields,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => &empty,
        };
        let litellm_params = object_of(fields.get("litellm_params"));
        let model_info = object_of(fields.get("model_info"));
        Self {
            model_name: text_field(fields, "model_name"),
            model: litellm_params.and_then(|params| text_field(params, "model")),
            tags: litellm_params.map_or_else(Vec::new, |params| tags_of(params.get("tags"))),
            model_info_id: model_info.and_then(|info| text_field(info, "id")),
            db_model: model_info
                .is_some_and(|info| matches!(info.get("db_model"), Some(Json::Bool(true)))),
            complexity_router_config: litellm_params
                .and_then(|params| params.get("complexity_router_config").cloned()),
        }
    }
}

#[must_use]
pub fn tuning_fingerprint(complexity_router_config: Option<&Json>) -> Option<String> {
    let empty = JsonObject::new();
    let fields = match complexity_router_config {
        Some(Json::Object(fields)) => fields,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => &empty,
    };
    validate_config_write(Some(&Json::Object(fields.clone()))).ok()?;
    let payload = tuning_payload::payload(fields)?;
    Some(hash_token(&compact_json(&Json::Object(payload))))
}

#[must_use]
pub fn uses_heuristic_v1(complexity_router_config: Option<&Json>) -> bool {
    let classifier_type = match complexity_router_config {
        Some(Json::Object(fields)) => fields.get("classifier_type"),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    };
    match classifier_type {
        None => true,
        Some(Json::Text(text)) => {
            matches!(text.as_str(), "heuristic" | "heuristic_first" | "hybrid")
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            false
        }
    }
}

#[must_use]
pub fn router_identity(deployment: &Deployment) -> Option<String> {
    if deployment.db_model
        && let Some(model_id) = deployment.model_info_id.as_deref()
        && !model_id.is_empty()
    {
        return Some(format!("db:{model_id}"));
    }
    let model_name = deployment.model_name.as_deref()?;
    if model_name.is_empty() {
        return None;
    }
    let mut tags = deployment.tags.clone();
    tags.sort();
    let pair = Json::List(vec![
        Json::Text(model_name.to_owned()),
        Json::List(tags.into_iter().map(Json::Text).collect()),
    ]);
    Some(format!("yaml:{}", compact_json(&pair)))
}

#[must_use]
pub fn heuristic_router_fingerprint(deployment: &Deployment) -> Option<(String, String)> {
    let model = deployment.model.as_deref()?;
    if !model.starts_with(AUTO_ROUTER_COMPLEXITY_PREFIX) {
        return None;
    }
    if !uses_heuristic_v1(deployment.complexity_router_config.as_ref()) {
        return None;
    }
    let identity = router_identity(deployment)?;
    let fingerprint = tuning_fingerprint(deployment.complexity_router_config.as_ref())?;
    Some((identity, fingerprint))
}

#[must_use]
pub fn snapshot_tuning_baselines(deployments: &[Deployment]) -> TuningBaselines {
    let mut baselines = TuningBaselines::new();
    for deployment in deployments {
        if let Some((identity, fingerprint)) = heuristic_router_fingerprint(deployment) {
            baselines.insert(identity, fingerprint);
        }
    }
    baselines
}

#[must_use]
pub fn is_mutable_tuned_candidate(candidate: &Deployment, baselines: &TuningBaselines) -> bool {
    let Some((identity, fingerprint)) = heuristic_router_fingerprint(candidate) else {
        return false;
    };
    changed_from_baseline(&identity, &fingerprint, baselines)
}

#[must_use]
pub fn mutable_tuned_identities(
    deployments: &[Deployment],
    baselines: &TuningBaselines,
) -> BTreeSet<String> {
    let mut identities = BTreeSet::new();
    for deployment in deployments {
        let Some((identity, fingerprint)) = heuristic_router_fingerprint(deployment) else {
            continue;
        };
        if changed_from_baseline(&identity, &fingerprint, baselines) {
            identities.insert(identity);
        }
    }
    identities
}

#[must_use]
pub fn tuning_limit_violation(held: usize, limit: Option<usize>) -> Option<String> {
    let allowed = limit?;
    if held <= allowed {
        return None;
    }
    Some(format!(
        "At most {allowed} auto-router(s) with changed heuristic scoring rules can be modified without an auto-router license. Keep this router on its recorded settings, or revert the other changed router to its baseline, or remove one of them. Selecting models does not use this allowance."
    ))
}

fn changed_from_baseline(identity: &str, fingerprint: &str, baselines: &TuningBaselines) -> bool {
    let baseline = baselines
        .get(identity)
        .cloned()
        .or_else(|| tuning_fingerprint(None));
    baseline.is_none_or(|recorded| recorded != fingerprint)
}

const fn object_of(value: Option<&Json>) -> Option<&JsonObject> {
    match value {
        Some(Json::Object(object)) => Some(object),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

fn text_field(fields: &JsonObject, key: &str) -> Option<String> {
    match fields.get(key) {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn tags_of(value: Option<&Json>) -> Vec<String> {
    match value {
        Some(Json::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                Json::Text(text) => Some(text.clone()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            })
            .collect(),
        Some(Json::Object(entries)) => entries.keys().cloned().collect(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_)) | None => Vec::new(),
    }
}

// json.dumps(..., separators=(",", ":")): dumps writes a backslash only inside a string, and only the separator spaces sit outside one
fn compact_json(value: &Json) -> String {
    let text = dumps(value);
    let mut out = String::with_capacity(text.len());
    let mut in_text = false;
    let mut escaped = false;
    for character in text.chars() {
        match character {
            '"' if !escaped => {
                in_text = !in_text;
                escaped = false;
                out.push(character);
            }
            '\\' => {
                escaped = !escaped;
                out.push(character);
            }
            ' ' if !in_text => {}
            _ => {
                escaped = false;
                out.push(character);
            }
        }
    }
    out
}

mod tuning_payload;
