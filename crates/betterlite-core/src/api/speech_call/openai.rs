use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::speech::call::{SpeechCall as NativeSpeechCall, audio_speech};
use betterlite_openai::speech::client::SpeechConnection;
use betterlite_openai::speech::request::{SpeechOptions, SpeechRequest};
use betterlite_openai::speech::result::SpeechTransport;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::api::speech_chat_bridge::SpeechCompletions;

use super::error::SpeechCallError;
use super::logging::{SpeechCallLogging, SpeechLoggerAdapter};
use super::operations::{SpeechConfigSource, SpeechOperations};
use super::service::{BranchCall, SpeechDispatch};

// OpenAI's SDK default when none of the base tiers is set
pub(crate) const DEFAULT_OPENAI_API_BASE: &str = "https://api.openai.com/v1";

// Python's `value or next`: an empty string falls through to the next tier
pub(crate) fn first_text<'value>(values: [Option<&'value str>; 4]) -> Option<&'value str> {
    values.into_iter().flatten().find(|value| !value.is_empty())
}

pub(crate) fn first_secret<'value>(
    values: [Option<&'value SecretString>; 5],
) -> Option<&'value SecretString> {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty())
}

impl<Transport, Completions, Configs, Operations>
    SpeechDispatch<'_, Transport, Completions, Configs, Operations>
where
    Transport: SpeechTransport + Sync,
    Completions: SpeechCompletions<Error = SpeechCallError> + Sync,
    Configs: SpeechConfigSource + Sync,
    Operations: SpeechOperations + Sync,
{
    // B514 over the credential chains speech applies before the OpenAI SDK call
    pub(crate) async fn openai_branch(
        &self,
        branch: BranchCall<'_, '_>,
        logging: &mut dyn SpeechCallLogging,
    ) -> Result<BinaryContent, SpeechCallError> {
        let Some(SpeechVoice::Name(name)) = &branch.voice else {
            return Err(SpeechCallError::BadRequest {
                message: String::from(
                    "'voice' is required to be passed as a string for OpenAI TTS",
                ),
            });
        };
        let settings = self.settings;
        let base = first_text([
            branch.resolved.api_base.as_deref(),
            settings.api_base.as_deref(),
            settings.openai_base_url.as_deref(),
            settings.openai_api_base.as_deref(),
        ])
        .unwrap_or(DEFAULT_OPENAI_API_BASE);
        let Some(key) = first_secret([
            branch.call.api_key.as_ref(),
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
        let organization = first_text([
            branch.call.organization.as_deref(),
            settings.organization.as_deref(),
            settings.openai_organization.as_deref(),
            None,
        ]);
        let project = first_text([
            branch.call.project.as_deref(),
            settings.project.as_deref(),
            settings.openai_project.as_deref(),
            None,
        ]);
        let timeout = self.timeout(branch.call)?;
        let options: SpeechOptions = serde_json::from_value(Value::Object(
            branch.optional_params.clone(),
        ))
        .map_err(|source| SpeechCallError::Internal {
            message: source.to_string(),
        })?;
        let connection = SpeechConnection {
            api_key: Some(key.clone()),
            api_base: Some(base.to_owned()),
            timeout,
            max_retries: branch.call.max_retries(settings),
            organization: organization.map(str::to_owned),
            project: project.map(str::to_owned),
            environment_organization: None,
            environment_project: None,
            client: branch.call.client.clone(),
            shared_session: branch.call.shared_session.clone(),
        };
        let request = SpeechRequest {
            model: &branch.resolved.model,
            input: &branch.call.input,
            voice: name,
            options: &options,
        };
        let mut logger = SpeechLoggerAdapter { logging };
        let response = audio_speech(
            NativeSpeechCall {
                request,
                connection,
                aspeech: branch.call.aspeech,
            },
            &mut logger,
            self.transport,
        )?
        .await?;
        self.content(response).await
    }
}
