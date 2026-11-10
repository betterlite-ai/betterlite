use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use betterlite_models::tiered_pricing::Scalar;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::de::IgnoredAny;

use super::error::BillingContextError;
use super::{CostResponse, IdentityRequest, ProviderInference, billing_model_name};

// What a deployment price lookup reads: the pricing flag, the router id and the request's logging params.
pub struct PricingQuery<'query> {
    pub custom_pricing: Option<bool>,
    pub router_model_id: Option<&'query str>,
    pub logging_params: Option<&'query IndexMap<String, Json>>,
}

// Ports _deployment_model_info: a router id's own raw entry wins even when empty, else the first nonnull model_info.
pub fn deployment_info(
    query: &PricingQuery<'_>,
    lookup: &dyn PriceLookup,
) -> Result<Option<PriceRecord>, BillingContextError> {
    if query.custom_pricing != Some(true) {
        return Ok(None);
    }
    if let Some(router_id) = query.router_model_id
        && let Some(entry) = lookup.raw_cost_map_entry(router_id)
    {
        return Ok(Some(entry));
    }
    let Some(params) = query.logging_params else {
        return Ok(None);
    };
    metadata_model_info(params)
}

// The first nonnull model_info under a nonempty metadata or litellm_metadata entry.
fn metadata_model_info(
    params: &IndexMap<String, Json>,
) -> Result<Option<PriceRecord>, BillingContextError> {
    for key in ["metadata", "litellm_metadata"] {
        let Some(metadata) = params.get(key) else {
            continue;
        };
        let Json::Object(fields) = metadata else {
            let present = match metadata {
                Json::Null => false,
                Json::Bool(flag) => *flag,
                Json::Number(number) => {
                    Decimal::from_str(&number.to_string()).is_ok_and(|amount| !amount.is_zero())
                }
                Json::Text(text) => !text.is_empty(),
                Json::List(items) => !items.is_empty(),
                Json::Object(_) => true,
            };
            if present {
                return Err(BillingContextError::MetadataShape { field: key });
            }
            continue;
        };
        if fields.is_empty() {
            continue;
        }
        match fields.get("model_info") {
            None | Some(Json::Null) => {}
            Some(Json::Object(info)) => return Ok(Some(price_record(info))),
            Some(_) => return Err(BillingContextError::ModelInfoShape),
        }
    }
    Ok(None)
}

// Ports pricing_entry_for_cost_calc: the deployment entry first, then the first candidate a soft lookup resolves.
pub fn pricing_entry<Inference>(
    request: &IdentityRequest<'_>,
    logging_params: Option<&IndexMap<String, Json>>,
    lookup: &dyn PriceLookup,
    inference: &Inference,
) -> Result<Option<(String, PriceRecord)>, BillingContextError>
where
    Inference: ProviderInference,
{
    let query = PricingQuery {
        custom_pricing: request.custom_pricing,
        router_model_id: request.router_model_id,
        logging_params,
    };
    if let Some(entry) = deployment_info(&query, lookup)?
        && let Some(key) = request.router_model_id.or(request.model)
    {
        return Ok(Some((key.to_owned(), entry)));
    }
    let selected = billing_model_name(request, lookup, inference);
    let from_response = match request.response {
        Some(CostResponse::Object { model, .. } | CostResponse::Dictionary { model }) => *model,
        None => None,
    };
    let candidates = [selected.as_deref(), from_response, request.model];
    for candidate in candidates.into_iter().flatten() {
        if candidate.is_empty() {
            continue;
        }
        let Some(resolved) = lookup
            .effective_model_info(candidate, request.custom_llm_provider)
            .filter(|record| !record.is_empty())
        else {
            continue;
        };
        let Some(Scalar::Text(key)) = resolved.get("key") else {
            return Err(BillingContextError::MissingKey {
                model: candidate.to_owned(),
            });
        };
        let raw = lookup
            .raw_cost_map_entry(key)
            .filter(|entry| !entry.is_empty());
        return Ok(Some((key.clone(), raw.unwrap_or(resolved))));
    }
    Ok(None)
}

pub(super) fn price_record(fields: &IndexMap<String, Json>) -> PriceRecord {
    fields
        .iter()
        .map(|(key, value)| (key.clone(), scalar(value)))
        .collect()
}

fn scalar(value: &Json) -> Scalar {
    match value {
        Json::Null => Scalar::Null,
        Json::Bool(flag) => Scalar::Bool(*flag),
        Json::Number(number) => Scalar::Number(number.clone()),
        Json::Text(text) => Scalar::Text(text.clone()),
        Json::List(_) => Scalar::Array(Vec::new()),
        Json::Object(_) => Scalar::Object(IgnoredAny),
    }
}
