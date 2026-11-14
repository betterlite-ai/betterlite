use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionError, ResolvedModel};

pub const PROXY_LLM_PROVIDER_FALLBACK: &str = "litellm_proxy";

// B174: get_llm_provider, injected so a rejected request never builds a resolution context here.
pub trait ProviderResolution: Send + Sync {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError>;
}

// B302: the deployments the router serves, in the order it published them.
pub trait DeploymentSnapshot: Send + Sync {
    fn model_list(&self) -> Result<Option<&[Json]>, SnapshotError>;
}

// The read fails here because LiteLLM's outer `except Exception` covers any failure around the model list.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("the router's model list could not be read")]
    Unreadable,
}

// A rate-limit or budget rejection's (model, provider); no secondary failure escapes this call.
#[must_use]
// ast-grep-ignore: no-long-names
pub fn resolve_llm_provider_for_rate_limit(
    model: Option<&str>,
    resolver: &dyn ProviderResolution,
    snapshot: &dyn DeploymentSnapshot,
) -> (String, String) {
    let Some(requested) = model.filter(|present| !present.is_empty()) else {
        return (String::new(), PROXY_LLM_PROVIDER_FALLBACK.to_owned());
    };
    match resolver.resolve(requested) {
        Ok(found) => resolved_fields(found, requested),
        Err(error) => {
            if let Some(alias) = resolve_from_router_alias(requested, resolver, snapshot) {
                return alias;
            }
            tracing::debug!(
                "rate_limiter_utils.resolve_llm_provider_for_rate_limit: could not resolve provider for model={requested}, falling back to {PROXY_LLM_PROVIDER_FALLBACK}. err={error}"
            );
            (requested.to_owned(), PROXY_LLM_PROVIDER_FALLBACK.to_owned())
        }
    }
}

// `resolved_model or model` and `custom_llm_provider or PROXY_LLM_PROVIDER_FALLBACK`, as LiteLLM returns them.
fn resolved_fields(found: ResolvedModel, original_model: &str) -> (String, String) {
    let ResolvedModel {
        model, provider, ..
    } = found;
    let resolved_model = if model.is_empty() {
        original_model.to_owned()
    } else {
        model
    };
    let slug = provider.slug();
    let provider_name = if slug.is_empty() {
        PROXY_LLM_PROVIDER_FALLBACK.to_owned()
    } else {
        slug.to_owned()
    };
    (resolved_model, provider_name)
}

// The underlying models of the deployments registered under `model`, in model_list order.
fn alias_candidates<'list>(model: &str, deployments: &'list [Json]) -> Vec<&'list str> {
    let mut candidates = Vec::new();
    for deployment in deployments {
        let Json::Object(row) = deployment else {
            continue;
        };
        let Some(Json::Text(name)) = row.get("model_name") else {
            continue;
        };
        if name != model {
            continue;
        }
        let Some(Json::Object(params)) = row.get("litellm_params") else {
            continue;
        };
        let Some(Json::Text(underlying)) = params.get("model") else {
            continue;
        };
        if !underlying.is_empty() {
            candidates.push(underlying.as_str());
        }
    }
    candidates
}

// _resolve_provider_from_router_alias: the first candidate that resolves wins, and every failure stays silent.
fn resolve_from_router_alias(
    model: &str,
    resolver: &dyn ProviderResolution,
    snapshot: &dyn DeploymentSnapshot,
) -> Option<(String, String)> {
    let deployments = snapshot.model_list().ok()??;
    for underlying in alias_candidates(model, deployments) {
        let Ok(found) = resolver.resolve(underlying) else {
            continue;
        };
        if found.provider.slug().is_empty() {
            continue;
        }
        return Some(resolved_fields(found, underlying));
    }
    None
}

mod tests;
