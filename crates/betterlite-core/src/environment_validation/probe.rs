use std::future::Future;

use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};
use secrecy::SecretString;

use super::error::EnvironmentValidationError;

pub struct ProbeCompletion<'request> {
    pub model: &'request str,
    pub user_message: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub max_tokens: Option<u32>,
}

// B158 wires the real calls; both methods answer only whether the call worked.
pub trait ModelProbe: Sync {
    type Failure;

    fn retrieve_openai_model(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send;

    fn complete(
        &self,
        request: &ProbeCompletion<'_>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send;
}

pub async fn valid_model<Probe>(
    probe: &Probe,
    model_sets: &ProviderModelSets,
    model: &str,
) -> Result<(), EnvironmentValidationError>
where
    Probe: ModelProbe,
{
    let is_openai_text_or_chat = [
        ProviderModelSet::OpenAiChatCompletion,
        ProviderModelSet::OpenAiTextCompletion,
    ]
    .iter()
    .any(|set| {
        model_sets
            .sets
            .get(set)
            .is_some_and(|names| names.contains(model))
    });
    let outcome = if is_openai_text_or_chat {
        probe.retrieve_openai_model(model).await
    } else {
        probe
            .complete(&ProbeCompletion {
                model,
                user_message: "Hello World",
                api_key: None,
                max_tokens: None,
            })
            .await
    };
    if outcome.is_err() {
        return Err(EnvironmentValidationError::BadRequest {
            model: model.to_owned(),
        });
    }
    Ok(())
}

pub async fn check_valid_key<Probe>(probe: &Probe, model: &str, api_key: &SecretString) -> bool
where
    Probe: ModelProbe,
{
    probe
        .complete(&ProbeCompletion {
            model,
            user_message: "Hey, how's it going?",
            api_key: Some(api_key),
            max_tokens: Some(10),
        })
        .await
        .is_ok()
}
