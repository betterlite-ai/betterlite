use std::future::Future;

use betterlite_config::sections::speech_call::Settings;
use betterlite_models::resolution::ResolvedModel;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::speech::call::{SpeechCall as NativeSpeechCall, audio_speech};
use betterlite_openai::speech::client::SpeechConnection;
use betterlite_openai::speech::request::{SpeechOptions, SpeechRequest};
use betterlite_openai::speech::result::SpeechTransport;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::api::speech_chat_bridge::SpeechCompletions;

use super::call::SpeechCall;
use super::error::SpeechCallError;
use super::logging::{SpeechCallLogging, SpeechLoggerAdapter};
use super::operations::{SpeechConfigSource, SpeechOperations};
use super::service::{BranchCall, SpeechDispatch};

// OpenAI's SDK default when none of the base tiers is set
pub(crate) const DEFAULT_OPENAI_API_BASE: &str = "https://api.openai.com/v1";

// Python's `value or next`: an empty string falls through to the next tier
pub(crate) fn first_text(values: [Option<&str>; 4]) -> Option<&str> {
    values
        .iter()
        .copied()
        .flatten()
        .find(|value| !value.is_empty())
}

pub(crate) fn first_secret(values: [Option<&SecretString>; 5]) -> Option<&SecretString> {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty())
}

// the credential tiers the OpenAI SDK call resolves, in speech's precedence order
struct OpenAiConfig<'config> {
    base: &'config str,
    key: &'config SecretString,
    organization: Option<String>,
    project: Option<String>,
}

fn openai_config<'config>(
    call: &'config SpeechCall,
    resolved: &'config ResolvedModel,
    settings: &'config Settings,
) -> Result<OpenAiConfig<'config>, SpeechCallError> {
    let base = first_text([
        resolved.api_base.as_deref(),
        settings.api_base.as_deref(),
        settings.openai_base_url.as_deref(),
        settings.openai_api_base.as_deref(),
    ])
    .unwrap_or(DEFAULT_OPENAI_API_BASE);
    let Some(key) = first_secret([
        call.api_key.as_ref(),
        settings.api_key.as_ref(),
        settings.openai_key.as_ref(),
        settings.openai_api_key.as_ref(),
        None,
    ]) else {
        return Err(SpeechCallError::Sdk {
            class: "OpenAIError",
            message: String::from(
                "The api_key client option must be set either by passing api_key to the client or by setting the OPENAI_API_KEY environment variable",
            ),
        });
    };
    let (organization, project) = openai_scopes(call, settings);
    Ok(OpenAiConfig {
        base,
        key,
        organization,
        project,
    })
}

// the call's organization and project over the two globals, then the two environment variables
pub(super) fn openai_scopes(
    call: &SpeechCall,
    settings: &Settings,
) -> (Option<String>, Option<String>) {
    let organization = first_text([
        call.organization.as_deref(),
        settings.organization.as_deref(),
        settings.openai_organization.as_deref(),
        None,
    ]);
    let project = first_text([
        call.project.as_deref(),
        settings.project.as_deref(),
        settings.openai_project.as_deref(),
        None,
    ]);
    (organization.map(str::to_owned), project.map(str::to_owned))
}

// the mapped options as the SDK call's typed options; a field outside it is a bug here, not a caller error
fn speech_options(optional_params: &Object) -> Result<SpeechOptions, SpeechCallError> {
    serde_json::from_value(Value::Object(optional_params.clone())).map_err(|source| {
        SpeechCallError::Internal {
            message: source.to_string(),
        }
    })
}

// the workspace denies multiple_inherent_impl, so the OpenAI branch lives on this trait
pub(crate) trait SpeechOpenAi<'service> {
    fn openai_branch<'call>(
        &'call self,
        branch: &'call BranchCall<'service, '_>,
        logging: &'call mut dyn SpeechCallLogging,
    ) -> impl Future<Output = Result<BinaryContent, SpeechCallError>> + Send + 'call;
}

impl<'service, Transport, Completions, Configs, Operations> SpeechOpenAi<'service>
    for SpeechDispatch<'service, Transport, Completions, Configs, Operations>
where
    Transport: SpeechTransport + Sync,
    Completions: SpeechCompletions<Error = SpeechCallError> + Sync,
    Configs: SpeechConfigSource + Sync,
    Operations: SpeechOperations + Sync,
{
    // B514 over the credential chains speech applies before the OpenAI SDK call
    async fn openai_branch(
        &self,
        branch: &BranchCall<'service, '_>,
        logging: &mut dyn SpeechCallLogging,
    ) -> Result<BinaryContent, SpeechCallError> {
        let Some(SpeechVoice::Name(name)) = &branch.voice else {
            return Err(SpeechCallError::BadRequest {
                message: String::from(
                    "'voice' is required to be passed as a string for OpenAI TTS",
                ),
            });
        };
        let config = openai_config(branch.call, branch.resolved, self.settings)?;
        let options = speech_options(&branch.optional_params)?;
        let connection = SpeechConnection {
            api_key: Some(config.key.clone()),
            api_base: Some(config.base.to_owned()),
            timeout: self.timeout(branch.call)?,
            max_retries: branch.call.max_retries(self.settings),
            organization: config.organization,
            project: config.project,
            environment_organization: None,
            environment_project: None,
            client: branch.call.client.clone(),
            shared_session: branch.call.shared_session.clone(),
        };
        let call = NativeSpeechCall {
            request: SpeechRequest {
                model: &branch.resolved.model,
                input: &branch.call.input,
                voice: name,
                options: &options,
            },
            connection,
            aspeech: branch.call.aspeech,
        };
        let response =
            audio_speech(call, &mut SpeechLoggerAdapter { logging }, self.transport)?.await?;
        Self::content(response).await
    }
}
