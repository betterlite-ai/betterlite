use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::openai_like_settings::Settings as OpenAiLikeSettings;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{JSONProviderRegistry, ProviderDescriptor};
use betterlite_openai::openai_base_config::Headers as OpenAiHeaders;
use secrecy::SecretString;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod calls;
mod capabilities;
mod cases;
mod tests;

use self::cases::{Call, CaseInput};

pub(crate) struct Op;

impl Helper for Op {
    type Input = CaseInput;

    fn run(input: &Self::Input) -> HelperResult {
        let registry = JSONProviderRegistry::load();
        let mut results: Vec<Value> = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(calls::run_call(call, &registry)?);
        }
        Ok(Value::Array(results))
    }
}

pub(super) fn split_model(model: &str) -> Result<(String, String), Box<HelperError>> {
    let Some((slug, rest)) = model.split_once('/') else {
        return Err(helper_error(
            "ValueError",
            format!("the case model {model} carries no provider prefix"),
        ));
    };
    Ok((slug.to_owned(), rest.to_owned()))
}

pub(super) fn provider_id(slug: &str) -> Result<ProviderId, Box<HelperError>> {
    slug.parse()
        .map_err(|error| helper_error("ValueError", format!("{slug}: {error}")))
}

pub(super) fn keys(call: &Call) -> (Option<SecretString>, Option<SecretString>) {
    (
        call.api_key.clone().map(SecretString::from),
        call.global_api_key.clone().map(SecretString::from),
    )
}

// the Responses policies carry the OpenAI crate's sorted header map
pub(super) fn sorted_headers(call: &Call) -> OpenAiHeaders {
    call.headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

pub(super) fn descriptor_for(
    call: &Call,
    registry: &JSONProviderRegistry,
    provider: ProviderId,
) -> Result<ProviderDescriptor, Box<HelperError>> {
    let Some(config) = &call.descriptor else {
        return registry.providers.get(&provider).cloned().ok_or_else(|| {
            helper_error("ValueError", format!("{provider} is not a JSON provider"))
        });
    };
    Ok(ProviderDescriptor {
        provider,
        config: config.clone(),
    })
}

pub(super) fn openai_like_settings(call: &Call) -> Result<OpenAiLikeSettings, Box<HelperError>> {
    ConfigLoader {
        lookup: call.environment.clone(),
    }
    .load("")
    .map(|config| config.sections.openai_like_settings)
    .map_err(|error| helper_error("ValueError", error.to_string()))
}

pub(super) fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("RuntimeError", error.to_string()))
}

pub(super) fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
