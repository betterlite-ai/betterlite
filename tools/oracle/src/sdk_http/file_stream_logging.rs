use std::collections::BTreeMap;
use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::file_stream_logging::logging::FileStreamLogger;
use betterlite_core::api::file_stream_logging::source::StreamSource;
use betterlite_core::api::file_stream_logging::{FileContentStreamingResponse, FileStreamSetup};
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_openai::files::records::FileContentRequest;
use betterlite_openai::files::{ClientArgs, FilesApi, get_openai_client, select_client};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

use fixtures::support::{FakeLogger, FaultyStream, WallClock};

mod fixtures;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    Drain,
    Fail,
    Close,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub file_id: String,
    pub model: String,
    pub scenario: Scenario,
    pub chunk_size: usize,
}

// The bytes the wrapper yielded, whether the fault surfaced, and what the logging callbacks saw
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub chunks: Vec<Vec<u8>>,
    pub raised: bool,
    pub success_calls: usize,
    pub failure_calls: usize,
    pub after_close_chunks: usize,
    pub capability: Option<String>,
}

impl CallKind for Call {
    fn validate(&self, _limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.chunk_size == 0 {
            return Err(SdkHttpError::Invalid(String::from(
                "the file stream needs a chunk_size of at least 1",
            )));
        }
        Ok(())
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        execute(self, api_base, env)
    }
}

async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let routing = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let tls = TlsPolicy::resolve(&config, env, None)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let wire = ClientPool::new(1_usize, TransportLimits::DEFAULT)
        .client(&tls, &routing)
        .map_err(|error| SdkHttpError::Client(Box::new(error)))?;
    let args = ClientArgs {
        api_key: Some(SecretString::from("sk-oracle")),
        api_base: Some(api_base.to_owned()),
        timeout: None,
        max_retries: None,
        organization: None,
    };
    let kind = get_openai_client(args, None, true)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let client =
        select_client(kind, true).map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let request = FileContentRequest {
        file_id: call.file_id.clone(),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    };
    let size = NonZeroUsize::new(call.chunk_size).ok_or_else(|| {
        SdkHttpError::Invalid(String::from(
            "the file stream needs a chunk_size of at least 1",
        ))
    })?;
    let opened = FilesApi {
        transport: wire,
        boundaries: Uuid::new_v4,
    }
    .file_content_streaming(&client, &request, Some(size))
    .await
    .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let logger = Arc::new(FakeLogger::default());
    let stream_logger: Arc<dyn FileStreamLogger> = Arc::clone(&logger);
    let fail_after = match call.scenario {
        Scenario::Fail => Some(1_usize),
        Scenario::Drain | Scenario::Close => None,
    };
    let mut stream = FileContentStreamingResponse::new(
        FileStreamSetup {
            source: StreamSource::Async(Box::new(FaultyStream {
                stream: opened.stream,
                served: 0_usize,
                fail_after,
            })),
            file_id: call.file_id.clone(),
            model: Some(call.model.clone()),
            custom_llm_provider: None,
            logger: Some(stream_logger),
        },
        Arc::new(WallClock),
    );
    let mut outcome = Outcome::default();
    match call.scenario {
        Scenario::Drain | Scenario::Fail => {
            let mut chunks = stream
                .async_iter()
                .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
            loop {
                match chunks.next_chunk().await {
                    Some(Ok(bytes)) => outcome.chunks.push(bytes),
                    Some(Err(_failure)) => {
                        outcome.raised = true;
                        break;
                    }
                    None => break,
                }
            }
        }
        Scenario::Close => {
            let first = {
                let mut chunks = stream
                    .async_iter()
                    .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
                chunks.next_chunk().await
            };
            if let Some(Ok(bytes)) = first {
                outcome.chunks.push(bytes);
            }
            stream
                .close()
                .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
            let mut empty = stream
                .sync_iter()
                .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
            while let Some(Ok(_bytes)) = empty.next_chunk() {
                outcome.after_close_chunks = outcome.after_close_chunks.saturating_add(1);
            }
            outcome.capability = stream.async_iter().err().map(|error| error.to_string());
        }
    }
    outcome.success_calls = logger.success.load(Ordering::SeqCst);
    outcome.failure_calls = logger.failure.load(Ordering::SeqCst);
    Ok(SdkResult::FileStreamLogging {
        value: Box::new(outcome),
    })
}
