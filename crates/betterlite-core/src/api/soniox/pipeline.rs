pub mod audio;
pub mod error;
pub mod exchange;
pub mod hidden;
pub mod job;

mod tests;

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::time::Duration;

use betterlite_openai::api::audio_file_input::AuthorizedAudioFiles;
use betterlite_openai::openai_base_config::{Headers, Object};

use self::error::PipelineError;
use self::job::{PipelineTarget, begin, finish};
use crate::api::api_boundaries::ApiLimits;
use crate::api::soniox::prepare::{PrepareInput, Prepared, prepare};
use crate::api::soniox::remote::transport::{RemoteTransport, Sleeper};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

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
    pub audio_files: &'deps (dyn AuthorizedAudioFiles + Sync),
    pub limits: ApiLimits,
    // multipart boundaries come from the caller so a test and the oracle script a fixed body
    pub boundaries: fn() -> String,
}

// The job the caller awaits: Send, so any worker can drive it after the handler runs the start
pub type JobFuture<'run> = Pin<Box<dyn Future<Output = Result<(), PipelineError>> + Send + 'run>>;

impl<P, S> SonioxPipeline<'_, P, S>
where
    P: RemoteTransport,
    S: Sleeper,
{
    // Both entry points run this job; the start before the first await owns the audio file.
    #[must_use]
    pub fn transcribe<'run, T>(
        &'run self,
        call: &'run PipelineCall<'_>,
        target: PipelineTarget<'run, '_>,
        supplied: Option<SuppliedClient<'run, T>>,
    ) -> JobFuture<'run>
    where
        T: RemoteTransport,
    {
        let prepared = match prepare(&PrepareInput {
            optional_params: call.optional_params,
            headers: call.headers.clone(),
            api_key: call.api_key,
            api_base: call.api_base,
            environment: call.environment,
        }) {
            Ok(prepared) => prepared,
            Err(error) => return Box::pin(ready(Err(PipelineError::from(error)))),
        };
        let expected = if call.atranscription {
            ClientKind::Asynchronous
        } else {
            ClientKind::Sync
        };
        let Some(client) = supplied.filter(|client| client.kind == expected) else {
            return start(self, call, prepared, target, self.provider);
        };
        start(self, call, prepared, target, client.transport)
    }
}

fn start<'run, P, S, T>(
    pipeline: &'run SonioxPipeline<'_, P, S>,
    call: &'run PipelineCall<'_>,
    prepared: Prepared,
    target: PipelineTarget<'run, '_>,
    transport: &'run T,
) -> JobFuture<'run>
where
    P: RemoteTransport,
    S: Sleeper,
    T: RemoteTransport,
{
    match begin(pipeline, call, prepared, target, transport) {
        Ok(started) => Box::pin(finish(started)),
        Err(error) => Box::pin(ready(Err(error))),
    }
}
