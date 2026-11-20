use betterlite_foundry::credentials::model_info::ModelInfoAccessors;
use betterlite_foundry::responses::decisions::AzureAiDecisionsConfig;
use betterlite_foundry::responses::url::{ResponsesUrlError, complete_url};
use betterlite_foundry::responses::{PROVIDER, SUPPORTS_NATIVE_WEBSOCKET};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Url(#[from] ResponsesUrlError),
}

// B557's Responses base URL, its Decisions route and the policy the Responses config inherits
pub struct Dispatch<'call> {
    pub call: Call<'call>,
}

pub enum Call<'call> {
    Url {
        accessors: ModelInfoAccessors<'call>,
        api_base: Option<&'call str>,
    },
    DecisionsUrl {
        api_base: &'call str,
        model: &'call str,
    },
    Policy,
}

pub enum Outcome {
    Url(String),
    DecisionsUrl(String),
    Policy {
        provider: ProviderId,
        native_websocket: bool,
    },
}

impl Dispatch<'_> {
    pub fn run(self) -> Result<Outcome, DispatchError> {
        Ok(match self.call {
            Call::Url {
                accessors,
                api_base,
            } => Outcome::Url(complete_url(accessors, api_base)?),
            Call::DecisionsUrl { api_base, model } => {
                Outcome::DecisionsUrl(AzureAiDecisionsConfig.get_complete_url(api_base, model))
            }
            Call::Policy => Outcome::Policy {
                provider: PROVIDER,
                native_websocket: SUPPORTS_NATIVE_WEBSOCKET,
            },
        })
    }
}
