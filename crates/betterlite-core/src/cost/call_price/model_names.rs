use betterlite_models::resolution::{ResolutionRequest, resolve};

use super::error::CallPriceError;
use super::{CallPriceEnv, CallPriceInput};

// The names a call is priced under; this is not the response-model selection of B1682.
pub struct ModelNames {
    pub provider: String,
    // The catalog key the raw lookup picked, else the model as given.
    pub model: String,
    // The model with its first slash component removed, kept for speech, transcription and Vertex.
    pub without_prefix: String,
}

// The model after repeated leading `provider/` chains collapse to one.
fn collapse<'name>(model: &'name str, prefix: &str) -> &'name str {
    let mut current = model;
    while let Some(remainder) = current.strip_prefix(prefix)
        && remainder.starts_with(prefix)
    {
        current = remainder;
    }
    current
}

pub fn select(
    model: &str,
    input: &CallPriceInput<'_>,
    env: &CallPriceEnv<'_>,
) -> Result<ModelNames, CallPriceError> {
    let mut current = model;
    let provider;
    let mut joined = model.to_owned();
    if let Some(supplied) = input.custom_llm_provider {
        let prefix = format!("{supplied}/");
        current = collapse(model, &prefix);
        let bare = current.strip_prefix(prefix.as_str());
        joined = bare.map_or_else(|| format!("{prefix}{current}"), |_| current.to_owned());
        if let Some(region) = input.region_name {
            let regional = format!("{prefix}{region}/{}", bare.unwrap_or(current));
            if env.lookup.raw_cost_map_entry(&regional).is_some() {
                joined = regional;
            }
        }
        provider = supplied.to_owned();
    } else {
        let resolved = resolve(
            &ResolutionRequest {
                model,
                custom_llm_provider: None,
                api_base: None,
                api_key: None,
            },
            env.resolution,
        )
        .map_err(|source| CallPriceError::Resolution { source })?;
        provider = resolved.provider.slug().to_owned();
    }
    let without_prefix = current.split_once('/').map_or(current, |(_, rest)| rest);
    let chosen = [joined.as_str(), current, without_prefix]
        .into_iter()
        .find(|name| env.lookup.raw_cost_map_entry(name).is_some())
        .unwrap_or(current);
    Ok(ModelNames {
        provider,
        model: chosen.to_owned(),
        without_prefix: without_prefix.to_owned(),
    })
}
