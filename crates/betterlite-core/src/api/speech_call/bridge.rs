use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::files::records::BinaryContent;
use indexmap::IndexMap;

use crate::api::speech_chat_bridge::arguments::SpeechBridgeArguments;
use crate::api::speech_chat_bridge::error::SpeechBridgeError;
use crate::api::speech_chat_bridge::{
    SpeechCompletionCall, SpeechCompletionOutput, SpeechCompletions, SpeechToCompletionBridge,
};

use super::error::SpeechCallError;
use super::logging::SpeechCallLogging;
use super::service::{BranchCall, SpeechDispatch};
use super::values;

// B1003's completion trait over the injected pipeline, whose error is already speech's
struct BridgeCompletions<'service, Completions> {
    completions: &'service Completions,
}

impl<Completions> SpeechCompletions for BridgeCompletions<'_, Completions>
where
    Completions: SpeechCompletions<Error = SpeechCallError> + Sync,
{
    type Error = SpeechCallError;

    fn speech_completion(
        &self,
        call: SpeechCompletionCall<'_>,
    ) -> impl Future<Output = Result<SpeechCompletionOutput, Self::Error>> + Send {
        self.completions.speech_completion(call)
    }
}

impl<Transport, Completions, Configs, Operations>
    SpeechDispatch<'_, Transport, Completions, Configs, Operations>
where
    Completions: SpeechCompletions<Error = SpeechCallError> + Sync,
{
    // B1003's bridge over the injected completion pipeline, with the headers speech passes it
    pub(crate) async fn bridge(
        &self,
        branch: &BranchCall<'_, '_>,
        logging: &mut dyn SpeechCallLogging,
    ) -> Result<BinaryContent, SpeechCallError> {
        let model = Json::Text(branch.resolved.model.clone());
        let input = Json::Text(branch.call.input.clone());
        let voice = voice_json(branch.voice.as_ref())?;
        let optional_params = Json::Object(values::json_object(&branch.optional_params)?);
        let litellm_params = Json::Object(values::json_object(&branch.litellm_params)?);
        let headers = match &branch.call.headers {
            Some(headers) => Json::Object(values::json_object(headers)?),
            None => Json::Object(IndexMap::new()),
        };
        let provider = Json::Text(branch.resolved.provider.slug().to_owned());
        let arguments = SpeechBridgeArguments {
            model: Some(&model),
            input: Some(&input),
            voice: voice.as_ref(),
            optional_params: Some(&optional_params),
            litellm_params: Some(&litellm_params),
            headers: Some(&headers),
            custom_llm_provider: Some(&provider),
        };
        let bridge = SpeechToCompletionBridge {
            completions: BridgeCompletions {
                completions: self.completions,
            },
        };
        let answer = bridge
            .speech(arguments, Some(logging.state()))
            .await
            .map_err(bridge_error)?;
        Ok(answer.content)
    }
}

fn bridge_error(error: SpeechBridgeError<SpeechCallError>) -> SpeechCallError {
    match error {
        SpeechBridgeError::Completion { source } => source,
        SpeechBridgeError::Transform(transform) => {
            if transform.error_fields().is_some() {
                SpeechCallError::BadRequest {
                    message: transform.to_string(),
                }
            } else {
                SpeechCallError::Internal {
                    message: transform.to_string(),
                }
            }
        }
        SpeechBridgeError::Required { name } => SpeechCallError::Internal {
            message: format!("{name} is required"),
        },
    }
}

fn voice_json(voice: Option<&SpeechVoice>) -> Result<Option<Json>, SpeechCallError> {
    match voice {
        None => Ok(None),
        Some(SpeechVoice::Name(name)) => Ok(Some(Json::Text(name.clone()))),
        Some(SpeechVoice::Options(options)) => {
            Ok(Some(Json::Object(values::json_object(options)?)))
        }
    }
}
