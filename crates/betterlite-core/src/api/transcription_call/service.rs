use std::error::Error;
use std::future::Future;
use std::sync::Mutex;

use self::run::execute;
use betterlite_config::sections::transcription_call::Settings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_openai::api::audio_file_input::decoder::{AudioDecoder, AudioDurationDecoder};
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
use super::error::{AsyncTranscriptionError, TranscriptionDispatchError, TranscriptionError};
use super::{
    cached_audio_response, error_type_name, public_duration_is_absent, store_hidden_duration,
};

mod branch;
mod logging;
mod run;

// transcription's whole outside world: the injected operations, the settings and the audio inputs
pub struct Transcriptions<'context, Dispatch> {
    pub dispatch: &'context Dispatch,
    pub settings: &'context Settings,
    pub resolution: ResolutionContext<'context>,
    pub bundled: &'context ModelCatalog,
    pub files: &'context (dyn AuthorizedAudioFiles + Sync),
    pub decoder: DurationDecoder<'context>,
    pub redactor: &'context Redactor,
    pub logging: Option<&'context Mutex<LoggingState>>,
    pub identify: fn() -> Uuid,
}

// the duration decoder the entry points measure with; the Sync bound keeps their futures Send
#[derive(Clone, Copy)]
pub enum DurationDecoder<'context> {
    Unavailable,
    Available(&'context (dyn AudioDurationDecoder + Sync)),
}

// one asynchronous call's inputs, shared by the first run and the retry the wrapper may make
struct AsyncCall<'call> {
    call: &'call TranscriptionCall,
    processed: &'call ProcessedAudioFile,
    input: &'call str,
}

// atranscription's synchronous prologue: the checked model, the cloned call and the processed file
struct AsyncPreparation {
    asynchronous: TranscriptionCall,
    model_name: String,
    input: String,
    processed: ProcessedAudioFile,
}

impl<Dispatch> Transcriptions<'_, Dispatch>
where
    Dispatch: TranscriptionDispatch,
{
    // transcription(model, file, ...): the sync call and its hidden duration, with the file consumed before any await
    pub fn transcription(
        &self,
        call: &TranscriptionCall,
        file: AudioFile<'_>,
    ) -> impl Future<Output = Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>>> + Send
    {
        let input = get_audio_file_name(&file);
        let processed = process_audio_file(file, self.files);
        async move {
            let processed = processed?;
            let answer = execute(self, call, &processed, &input).await?;
            Ok(self.with_measured_duration(answer, &processed))
        }
    }

    // atranscription(*args, **kwargs): KeyError("model") before the try, then the sync call and its checks
    pub fn atranscription(
        &self,
        model: Option<&str>,
        call: &TranscriptionCall,
        file: AudioFile<'_>,
        file_from_kwargs: bool,
    ) -> impl Future<Output = Result<TranscriptionResponse, AsyncTranscriptionError>> + Send {
        let preparation = self.prepare_async(model, call, file);
        async move {
            let prepared = preparation?;
            self.finish_async(prepared, file_from_kwargs).await
        }
    }

    // atranscription's synchronous part: args[0] is checked first, then the file is read and a failure mapped
    fn prepare_async(
        &self,
        model: Option<&str>,
        call: &TranscriptionCall,
        file: AudioFile<'_>,
    ) -> Result<AsyncPreparation, AsyncTranscriptionError> {
        let Some(model_name) = model else {
            return Err(AsyncTranscriptionError::MissingModel);
        };
        let mut asynchronous = call.clone();
        model_name.clone_into(&mut asynchronous.model);
        asynchronous.atranscription = Some(Json::Bool(true));
        let input = get_audio_file_name(&file);
        let processed = match process_audio_file(file, self.files) {
            Ok(processed) => processed,
            Err(error) => {
                let failure = TranscriptionError::<Dispatch::Error>::Audio(error);
                return Err(self.mapped_error(&failure, &asynchronous, model_name));
            }
        };
        Ok(AsyncPreparation {
            asynchronous,
            model_name: model_name.to_owned(),
            input,
            processed,
        })
    }

    // atranscription's awaited part: the sync call, the wrapper's final checks and the hidden duration
    async fn finish_async(
        &self,
        prepared: AsyncPreparation,
        file_from_kwargs: bool,
    ) -> Result<TranscriptionResponse, AsyncTranscriptionError> {
        let async_call = AsyncCall {
            call: &prepared.asynchronous,
            processed: &prepared.processed,
            input: &prepared.input,
        };
        let answer = self.transcribed(&async_call, &prepared.model_name).await?;
        let mut response = self
            .final_response(answer, &async_call, &prepared.model_name)
            .await?;
        if file_from_kwargs
            && public_duration_is_absent(&response)
            && let Some(duration) = measured_duration(&prepared.processed, self.files, self.decoder)
        {
            store_hidden_duration(&mut response, duration);
        }
        Ok(response)
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

    // one sync stage, with its errors mapped through the outer wrapper's own rules
    async fn transcribed(
        &self,
        async_call: &AsyncCall<'_>,
        model: &str,
    ) -> Result<TranscriptionAnswer, AsyncTranscriptionError> {
        execute(
            self,
            async_call.call,
            async_call.processed,
            async_call.input,
        )
        .await
        .map_err(|error| self.mapped_error(&error, async_call.call, model))
    }

    // the wrapper's isinstance chain: a reply, a cached dict, or one retry and the final type check
    async fn final_response(
        &self,
        answer: TranscriptionAnswer,
        async_call: &AsyncCall<'_>,
        model: &str,
    ) -> Result<TranscriptionResponse, AsyncTranscriptionError> {
        match self.with_measured_duration(answer, async_call.processed) {
            TranscriptionAnswer::Reply(reply) => Ok(reply.response),
            TranscriptionAnswer::Mapping(mapping) => cached_audio_response(mapping)
                .map_err(|error| self.mapped_error(&error, async_call.call, model)),
            TranscriptionAnswer::Unexpected(_) => {
                let second = self.transcribed(async_call, model).await?;
                match self.with_measured_duration(second, async_call.processed) {
                    TranscriptionAnswer::Reply(reply) => Ok(reply.response),
                    TranscriptionAnswer::Mapping(_) => {
                        let failure = TranscriptionError::<Dispatch::Error>::InvalidResponse {
                            python_type: "<class 'dict'>",
                        };
                        Err(self.mapped_error(&failure, async_call.call, model))
                    }
                    TranscriptionAnswer::Unexpected(value) => {
                        let failure = TranscriptionError::<Dispatch::Error>::InvalidResponse {
                            python_type: error_type_name(&value),
                        };
                        Err(self.mapped_error(&failure, async_call.call, model))
                    }
                }
            }
        }
    }

    // the outer wrapper maps through exception_type(model=model, ...) with "openai" as the provider fallback
    fn mapped_error<SourceError>(
        &self,
        error: &TranscriptionError<SourceError>,
        call: &TranscriptionCall,
        model: &str,
    ) -> AsyncTranscriptionError
    where
        SourceError: Error + TranscriptionDispatchError,
    {
        AsyncTranscriptionError::mapped(
            error,
            model,
            &self.attributed_provider(call),
            self.redactor,
        )
    }
}

// calculate_request_duration over the processed bytes, as the request duration entry builds them
fn measured_duration(
    processed: &ProcessedAudioFile,
    files: &(dyn AuthorizedAudioFiles + Sync),
    decoder: DurationDecoder<'_>,
) -> Option<Number> {
    let mut file = AudioFile::Tuple(AudioTuple {
        filename: &processed.filename,
        content: Some(TupleContent::Bytes(&processed.file_content)),
    });
    let audio_decoder = match decoder {
        DurationDecoder::Unavailable => AudioDecoder::Unavailable,
        DurationDecoder::Available(available) => AudioDecoder::Available(available),
    };
    let duration = calculate_request_duration(&mut file, files, audio_decoder)?;
    Number::from_f64(duration.to_f64()?)
}
