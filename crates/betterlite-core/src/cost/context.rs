use std::fmt::Debug;

use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use betterlite_models::resolution::ProviderIdentity;
use betterlite_models::tiered_pricing::Scalar;
use indexmap::IndexMap;

pub mod error;
pub mod ocr;
pub mod pricing;
pub mod response_time;
pub mod storage;

mod tests;

const NON_TOKEN_RATE_FIELDS: [&str; 5] = [
    "cost_per_second",
    "input_cost_per_second",
    "output_cost_per_second",
    "input_cost_per_query",
    "tiered_pricing",
];

// providers.json registers these slugs, but litellm.types.utils.LlmProviders does not, and the source asks that set.
const REGISTRY_ONLY_PROVIDERS: [ProviderId; 8] = [
    ProviderId::Abliteration,
    ProviderId::Aihubmix,
    ProviderId::Crusoe,
    ProviderId::Empiriolabs,
    ProviderId::Gmi,
    ProviderId::Llamagate,
    ProviderId::Sarvam,
    ProviderId::Veniceai,
];

// get_llm_provider's inference step, injected so this policy reads no global catalog (B174).
pub trait ProviderInference {
    type Failure: Debug;

    fn inferred_provider(&self, model: &str) -> Result<ProviderIdentity, Self::Failure>;
}

// A typed response exposes model, _hidden_params and _response_ms as attributes; a dictionary exposes only its keys.
pub enum CostResponse<'response> {
    Object {
        model: Option<&'response str>,
        hidden_params: Option<&'response IndexMap<String, Json>>,
        response_ms: Option<&'response Json>,
    },
    Dictionary {
        model: Option<&'response str>,
    },
}

pub struct IdentityRequest<'request> {
    pub model: Option<&'request str>,
    pub response: Option<&'request CostResponse<'request>>,
    pub base_model: Option<&'request str>,
    pub custom_pricing: Option<bool>,
    pub custom_llm_provider: Option<&'request str>,
    pub router_model_id: Option<&'request str>,
    pub region_name: Option<&'request str>,
}

// Ports _select_model_name_for_cost_calc: the model whose prices bill this call, or no name when nothing is known.
#[must_use]
pub fn billing_model_name<Inference>(
    request: &IdentityRequest<'_>,
    lookup: &dyn PriceLookup,
    inference: &Inference,
) -> Option<String>
where
    Inference: ProviderInference,
{
    let provider_name = provider_for_cost(request.model, request.custom_llm_provider, inference);
    let (response_model, hidden_params) = response_parts(request.response);
    let provider_response_model = hidden_text(hidden_params, "provider_response_model");
    let explicit_pricing = request.custom_pricing == Some(true) || request.base_model.is_some();
    let priced_from_response = provider_response_model.is_some() || response_model.is_some();
    let priced_region = if explicit_pricing || !priced_from_response {
        None
    } else {
        hidden_text(hidden_params, "region_name").or(request.region_name)
    };
    let selected = selected_name(
        request,
        lookup,
        provider_response_model,
        response_model,
        hidden_params,
    );
    let (Some(name), Some(provider)) = (selected.as_deref(), provider_name.as_deref()) else {
        return selected;
    };
    if has_known_provider(name) {
        return selected;
    }
    let prefix = priced_region.map_or_else(
        || provider.to_owned(),
        |region| format!("{provider}/{region}"),
    );
    Some(strip_unregistered_head(
        &format!("{prefix}/{name}"),
        priced_region,
        lookup,
    ))
}

fn provider_for_cost<Inference>(
    model: Option<&str>,
    custom_llm_provider: Option<&str>,
    inference: &Inference,
) -> Option<String>
where
    Inference: ProviderInference,
{
    if let Some(provider) = custom_llm_provider {
        return Some(provider.to_owned());
    }
    match inference.inferred_provider(model?) {
        Ok(provider) => Some(provider.slug().to_owned()),
        Err(failure) => {
            tracing::debug!(?failure, "Error inferring custom_llm_provider");
            None
        }
    }
}

fn selected_name(
    request: &IdentityRequest<'_>,
    lookup: &dyn PriceLookup,
    provider_response_model: Option<&str>,
    response_model: Option<&str>,
    hidden_params: Option<&IndexMap<String, Json>>,
) -> Option<String> {
    let mut selected = if request.custom_pricing == Some(true) {
        router_name(request, lookup)
    } else if let Some(base_model) = request.base_model {
        Some(base_model.to_owned())
    } else if let Some(provider_model) = provider_response_model {
        Some(provider_model.to_owned())
    } else if response_model.is_none() {
        hidden_text(hidden_params, "model").map(str::to_owned)
    } else {
        None
    };
    if selected.is_none() {
        selected = response_model.map(str::to_owned);
    }
    if selected.is_none() {
        selected = request.model.map(str::to_owned);
    }
    selected
}

fn router_name(request: &IdentityRequest<'_>, lookup: &dyn PriceLookup) -> Option<String> {
    let prices = request
        .router_model_id
        .and_then(|router_id| lookup.raw_cost_map_entry(router_id))
        .is_some_and(|entry| entry_prices_anything(&entry));
    if prices {
        request.router_model_id.map(str::to_owned)
    } else {
        request.model.map(str::to_owned)
    }
}

fn entry_prices_anything(entry: &PriceRecord) -> bool {
    entry.iter().any(|(field, value)| {
        !matches!(value, Scalar::Null)
            && (NON_TOKEN_RATE_FIELDS.contains(&field.as_str())
                || (field.contains("cost_per") && field.contains("token")))
    })
}

fn hidden_text<'hidden>(
    hidden_params: Option<&'hidden IndexMap<String, Json>>,
    key: &str,
) -> Option<&'hidden str> {
    match hidden_params?.get(key) {
        Some(Json::Text(text)) if !text.is_empty() => Some(text),
        _ => None,
    }
}

fn has_known_provider(model: &str) -> bool {
    let first = model.split('/').next().unwrap_or(model);
    first
        .parse::<ProviderId>()
        .is_ok_and(|provider| !REGISTRY_ONLY_PROVIDERS.contains(&provider))
}

fn strip_unregistered_head(
    model: &str,
    region_name: Option<&str>,
    lookup: &dyn PriceLookup,
) -> String {
    let segments: Vec<&str> = model.split('/').collect();
    if lookup
        .raw_cost_map_entry(&segments.get(1..).unwrap_or_default().join("/"))
        .is_some()
    {
        return model.to_owned();
    }
    let region_head = region_name.is_some_and(|region| segments.get(1) == Some(&region));
    let head_len = if region_head { 2 } else { 1 };
    let Some((head, tail)) = segments.split_at_checked(head_len) else {
        return model.to_owned();
    };
    if tail.is_empty() {
        return model.to_owned();
    }
    let strippable = tail
        .iter()
        .position(|segment| has_known_provider(segment))
        .unwrap_or(tail.len());
    let last_start = strippable.min(tail.len().saturating_sub(1));
    for start in 0..=last_start {
        let candidate = format!(
            "{}/{}",
            head.join("/"),
            tail.get(start..).unwrap_or_default().join("/")
        );
        if lookup.raw_cost_map_entry(&candidate).is_some() {
            return candidate;
        }
    }
    model.to_owned()
}

const fn response_parts<'response>(
    response: Option<&'response CostResponse<'response>>,
) -> (
    Option<&'response str>,
    Option<&'response IndexMap<String, Json>>,
) {
    match response {
        Some(CostResponse::Object {
            model,
            hidden_params,
            ..
        }) => (*model, *hidden_params),
        Some(CostResponse::Dictionary { model }) => (*model, None),
        None => (None, None),
    }
}
