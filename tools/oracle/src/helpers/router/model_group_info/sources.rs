use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};

use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::model_group_info::sources::{
    DeploymentInfo, ProviderAnswer, ProviderFailure, ProviderRequest, ProviderResolver,
};

use super::error::Failure;
use super::input::ProviderError;

#[derive(Debug, thiserror::Error)]
#[error("the case asked for a failing deployment metadata lookup")]
pub(crate) struct Unavailable;

// get_deployment_model_info as the case declares it, or a failing lookup when it asked for one.
pub(crate) struct CaseInfo {
    answers: BTreeMap<String, JsonObject>,
    failing: bool,
}

impl CaseInfo {
    pub(crate) fn new(entries: &JsonObject, failing: bool) -> Result<Self, Failure> {
        let answers = entries
            .iter()
            .map(|(id, value)| Ok((id.clone(), object_answer(value, id)?)))
            .collect::<Result<BTreeMap<String, JsonObject>, Failure>>()?;
        Ok(Self { answers, failing })
    }
}

fn object_answer(value: &Json, id: &str) -> Result<JsonObject, Failure> {
    let Json::Object(fields) = value else {
        return Err(Failure::Metadata(id.to_owned()));
    };
    Ok(fields.clone())
}

impl DeploymentInfo for CaseInfo {
    type Error = Unavailable;

    fn info(
        &self,
        model_id: &str,
        _model_name: &str,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<JsonObject>, Unavailable>> + Send {
        if self.failing {
            return ready(Err(Unavailable));
        }
        ready(Ok(self.answers.get(model_id).cloned()))
    }
}

// get_llm_provider for the prefixed models the cases declare, with a forced failure when the case asks for one.
pub(crate) struct CaseProvider {
    pub(crate) error: ProviderError,
}

const KNOWN_PROVIDERS: [&str; 4] = ["openai", "anthropic", "azure", "gemini"];

#[derive(Debug, thiserror::Error)]
#[error("the case asked for a provider error")]
pub(crate) struct ProviderUnavailable;

impl ProviderResolver for CaseProvider {
    type Error = ProviderUnavailable;

    fn resolve_provider(
        &self,
        request: &ProviderRequest<'_>,
    ) -> Result<ProviderAnswer, ProviderFailure<ProviderUnavailable>> {
        match self.error {
            ProviderError::None => resolve_prefixed(request.model),
            ProviderError::BadRequest => Err(missing_provider(request.model)),
            ProviderError::Other => Err(ProviderFailure::Other(ProviderUnavailable)),
        }
    }
}

fn resolve_prefixed(model: &str) -> Result<ProviderAnswer, ProviderFailure<ProviderUnavailable>> {
    match model.split_once('/') {
        Some((provider, name)) if KNOWN_PROVIDERS.contains(&provider) => Ok(ProviderAnswer {
            litellm_model: name.to_owned(),
            provider: provider.to_owned(),
        }),
        Some(_) | None => Err(missing_provider(model)),
    }
}

fn missing_provider(model: &str) -> ProviderFailure<ProviderUnavailable> {
    ProviderFailure::BadRequest(format!(
        "LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model={model}"
    ))
}

// get_supported_openai_params as the case declares it, by the provider-qualified model it is asked about.
pub(crate) struct CaseParameters {
    answers: BTreeMap<String, Option<Vec<String>>>,
}

impl CaseParameters {
    pub(crate) fn new(entries: &JsonObject) -> Result<Self, Failure> {
        let mut answers = BTreeMap::new();
        for (key, value) in entries {
            let answer = match value {
                Json::Null => None,
                Json::List(_) => Some(texts(value, key)?),
                Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                    return Err(Failure::Parameters(key.clone()));
                }
            };
            answers.insert(key.clone(), answer);
        }
        Ok(Self { answers })
    }
}

impl SupportedParameterSource for CaseParameters {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        let key = custom_llm_provider.map_or_else(
            || model.to_owned(),
            |provider| format!("{provider}/{model}"),
        );
        Ok(self.answers.get(&key).cloned().flatten())
    }
}

fn texts(value: &Json, key: &str) -> Result<Vec<String>, Failure> {
    let Json::List(items) = value else {
        return Err(Failure::Parameters(key.to_owned()));
    };
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let Json::Text(name) = item else {
            return Err(Failure::Parameters(key.to_owned()));
        };
        names.push(name.clone());
    }
    Ok(names)
}
