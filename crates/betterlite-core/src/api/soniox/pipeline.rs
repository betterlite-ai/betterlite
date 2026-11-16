pub mod audio;
pub mod client;
pub mod error;
pub mod exchange;
pub mod hidden;
pub mod job;

mod tests;

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_openai::api::audio_file_input::{AudioFile, AuthorizedAudioFiles};
use betterlite_openai::openai_base_config::{Headers, Object};

use self::client::{ClientKind, SuppliedClient};
use self::error::PipelineError;
use self::job::{PipelineTarget, execute};
use crate::api::api_boundaries::ApiLimits;
use crate::api::soniox::prepare::logging::SonioxLogger;
use crate::api::soniox::prepare::{PrepareInput, prepare};
use crate::api::soniox::records::response::SonioxTranscriptionResponse;
use crate::api::soniox::remote::transport::{RemoteTransport, Sleeper};

pub struct PipelineCall<'call> {
    pub model: &'call str,
    pub optional_params: &'call Object,
    pub headers: Headers,
    pub api_key: Option<&'call str>,
    pub api_base: Option<&'call str>,
    pub environment: &'call BTreeMap<String, String>,
    pub timeout: Duration,
    pub atranscription: bool,
}

pub struct SonioxPipeline<'deps, P, S> {
    pub provider: &'deps P,
    pub sleeper: &'deps S,
    pub audio_files: &'deps dyn AuthorizedAudioFiles,
    pub limits: ApiLimits,
    // multipart boundaries come from the caller so a test and the oracle script a fixed body
    pub boundaries: fn() -> String,
}

impl<P, S> SonioxPipeline<'_, P, S>
where
    P: RemoteTransport,
    S: Sleeper,
{
    // LiteLLM's two entry points differ only in which client they keep, so both run this one job
    pub async fn transcribe<T>(
        &self,
        call: &PipelineCall<'_>,
        target: PipelineTarget<'_, '_>,
        supplied: Option<SuppliedClient<'_, T>>,
    ) -> Result<(), PipelineError>
    where
        T: RemoteTransport,
    {
        let prepared = prepare(&PrepareInput {
            optional_params: call.optional_params,
            headers: call.headers.clone(),
            api_key: call.api_key,
            api_base: call.api_base,
            environment: call.environment,
        })?;
        let expected = if call.atranscription {
            ClientKind::Async
        } else {
            ClientKind::Sync
        };
        match supplied.filter(|client| client.kind == expected) {
            Some(client) => execute(self, call, &prepared, target, client.transport).await,
            None => execute(self, call, &prepared, target, self.provider).await,
        }
    }
}
