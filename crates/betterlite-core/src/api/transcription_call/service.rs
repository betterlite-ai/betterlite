use std::sync::Mutex;

use betterlite_config::sections::transcription_call::Settings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_openai::api::audio_file_input::decoder::AudioDecoder;
use betterlite_openai::api::audio_file_input::duration::calculate_request_duration;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioTuple, AuthorizedAudioFiles, ProcessedAudioFile, TupleContent,
    get_audio_file_name, process_audio_file,
};
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_redact::secret::Redactor;
use num_traits::ToPrimitive as _;
use serde_json::Number;
use uuid::Uuid;

use super::call::TranscriptionCall;
use super::dispatch::{TranscriptionAnswer, TranscriptionDispatch};
use super::error::{AsyncTranscriptionError, TranscriptionError};
use super::{
    public_duration_is_absent, python_type_name, store_hidden_duration,
    transcription_response_from_cached_dict,
};

mod logging;
mod run;

// transcription's whole outside world: the injected operations, the settings and the audio inputs
pub struct Transcriptions<'context, Dispatch> {
    pub dispatch: &'context Dispatch,
    pub settings: &'context Settings,
    pub resolution: ResolutionContext<'context>,
    pub bundled: &'context ModelCatalog,
    pub files: &'context dyn AuthorizedAudioFiles,
    pub decoder: AudioDecoder<'context>,
    pub redactor: &'context Redactor,
    pub logging: Option<&'context Mutex<LoggingState>>,
    pub identify: fn() -> Uuid,
}

impl<Dispatch> Transcriptions<'_, Dispatch>
where
    Dispatch: TranscriptionDispatch,
{
    // transcription(model, file, ...): the sync entry, which also measures the request duration
    pub async fn transcription(
        &self,
        call: &TranscriptionCall,
        file: AudioFile<'_>,
    ) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
        let input = get_audio_file_name(&file);
        let processed = process_audio_file(file, self.files)?;
        self.transcription_answer(call, &processed, &input).await
    }

    // atranscription(*args, **kwargs): KeyError("model") before the try, then the sync call and its checks
    pub async fn atranscription(
        &self,
        model: Option<&str>,
        call: &TranscriptionCall,
        file: AudioFile<'_>,
        file_from_kwargs: bool,
    ) -> Result<TranscriptionResponse, AsyncTranscriptionError<Dispatch::Error>> {
        let Some(model) = model else {
            return Err(AsyncTranscriptionError::MissingModel);
        };
        let mut asynchronous = call.clone();
        model.clone_into(&mut asynchronous.model);
        asynchronous.atranscription = Some(Json::Bool(true));
        let provider = self.attributed_provider(&asynchronous);
        let input = get_audio_file_name(&file);
        let processed = match process_audio_file(file, self.files) {
            Ok(processed) => processed,
            Err(error) => {
                let failure = TranscriptionError::<Dispatch::Error>::Audio(error);
                return Err(AsyncTranscriptionError::mapped(
                    &failure,
                    model,
                    &provider,
                    self.redactor,
                ));
            }
        };
        let mut response = match self
            .transcription_answer(&asynchronous, &processed, &input)
            .await
            .map_err(|error| {
                AsyncTranscriptionError::mapped(&error, model, &provider, self.redactor)
            })? {
            TranscriptionAnswer::Reply(reply) => reply.response,
            TranscriptionAnswer::Mapping(mapping) => {
                match transcription_response_from_cached_dict(mapping) {
                    Ok(response) => response,
                    Err(error) => {
                        return Err(AsyncTranscriptionError::mapped(
                            &error,
                            model,
                            &provider,
                            self.redactor,
                        ));
                    }
                }
            }
            TranscriptionAnswer::Unexpected(_) => {
                let second = self
                    .transcription_answer(&asynchronous, &processed, &input)
                    .await
                    .map_err(|error| {
                        AsyncTranscriptionError::mapped(&error, model, &provider, self.redactor)
                    })?;
                match second {
                    TranscriptionAnswer::Reply(reply) => reply.response,
                    TranscriptionAnswer::Mapping(_) => {
                        let failure = TranscriptionError::<Dispatch::Error>::InvalidResponse {
                            python_type: "<class 'dict'>",
                        };
                        return Err(AsyncTranscriptionError::mapped(
                            &failure,
                            model,
                            &provider,
                            self.redactor,
                        ));
                    }
                    TranscriptionAnswer::Unexpected(value) => {
                        let failure = TranscriptionError::<Dispatch::Error>::InvalidResponse {
                            python_type: python_type_name(&value),
                        };
                        return Err(AsyncTranscriptionError::mapped(
                            &failure,
                            model,
                            &provider,
                            self.redactor,
                        ));
                    }
                }
            }
        };
        if file_from_kwargs
            && public_duration_is_absent(&response)
            && let Some(duration) = measured_duration(&processed, self.files, self.decoder)
        {
            store_hidden_duration(&mut response, duration);
        }
        Ok(response)
    }

    // the sync call's body, before the outer wrapper looks at what it returned
    async fn transcription_answer(
        &self,
        call: &TranscriptionCall,
        processed: &ProcessedAudioFile,
        input: &str,
    ) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
        let answer = self.run(call, processed, input).await?;
        Ok(self.with_measured_duration(answer, processed))
    }

    // both sync results and awaited async results get the duration, stored hidden only
    fn with_measured_duration(
        &self,
        answer: TranscriptionAnswer,
        processed: &ProcessedAudioFile,
    ) -> TranscriptionAnswer {
        match answer {
            TranscriptionAnswer::Reply(mut reply) => {
                if public_duration_is_absent(&reply.response)
                    && let Some(duration) = measured_duration(processed, self.files, self.decoder)
                {
                    store_hidden_duration(&mut reply.response, duration);
                }
                TranscriptionAnswer::Reply(reply)
            }
            TranscriptionAnswer::Mapping(mapping) => TranscriptionAnswer::Mapping(mapping),
            TranscriptionAnswer::Unexpected(value) => TranscriptionAnswer::Unexpected(value),
        }
    }

    // the outer wrapper's get_llm_provider(model=model, api_base=kwargs.get("api_base")) and its "openai" fallback
    fn attributed_provider(&self, call: &TranscriptionCall) -> String {
        let request = ResolutionRequest {
            model: &call.model,
            custom_llm_provider: None,
            api_base: call.api_base.as_deref(),
            api_key: None,
        };
        resolve(&request, &self.resolution).map_or_else(
            |_error| String::from("openai"),
            |resolved| resolved.provider.slug().to_owned(),
        )
    }
}

// calculate_request_duration over the processed bytes, as the request duration entry builds them
fn measured_duration(
    processed: &ProcessedAudioFile,
    files: &dyn AuthorizedAudioFiles,
    decoder: AudioDecoder<'_>,
) -> Option<Number> {
    let mut file = AudioFile::Tuple(AudioTuple {
        filename: &processed.filename,
        content: Some(TupleContent::Bytes(&processed.file_content)),
    });
    let duration = calculate_request_duration(&mut file, files, decoder)?;
    Number::from_f64(duration.to_f64()?)
}
