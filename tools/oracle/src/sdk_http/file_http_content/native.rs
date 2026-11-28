use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::file_http_content::config::{ContentParams, FileContentConfig, Headers};
use betterlite_core::api::file_http_content::error::ContentError;
use betterlite_core::api::file_http_content::service::buffered::retrieve_file_content;
use betterlite_core::api::file_http_content::service::streaming::retrieve_content_streaming;
use betterlite_core::api::file_http_content::service::{
    ClientSource, ContentDeps, ContentTransport, PreparedContent, RetrieveCall,
};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{BinaryContent, FileContentRequest};
use futures_util::StreamExt as _;
use indexmap::IndexMap;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{Fixture, ProviderContent, source};
use super::{BufferedContent, Call, ContentValue, Failure, Mode, Outcome, StreamedContent};

// The production service over the mock's base: the same provider config, pooled client and pre-call a request runs
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let params = ContentParams {
        api_base: Some(api_base.to_owned()),
        api_key: None,
    };
    let provider = ProviderContent::new(env, &params);
    let origin = source(&config, env)?;
    let mut fixture = Fixture::new(call.mode.logging_call_type())?;
    let request = file_content_request(call);
    let caller = caller_headers(call);
    let retrieve = RetrieveCall {
        request: &request,
        params: &params,
        headers: &caller,
        timeout: call.timeout_seconds.map(Duration::from_secs),
    };
    let deps = ContentDeps {
        config: &provider,
        source: &origin,
    };
    let prepared = fixture.prepare(&deps, &retrieve)?;
    let outcome = match call.mode {
        Mode::Sync | Mode::Asynchronous => {
            let content = retrieve_file_content(prepared, &mut fixture.state).await;
            content_outcome(content.map(buffered))?
        }
        Mode::Streaming => streamed(prepared, chunk_size(call)?, &mut fixture.state).await?,
    };
    Ok(SdkResult::FileHttpContent {
        value: Box::new(outcome),
    })
}

fn file_content_request(call: &Call) -> FileContentRequest {
    FileContentRequest {
        file_id: call.file_id.clone(),
        extra_headers: call.extra_headers.clone().unwrap_or_default(),
        extra_body: call.extra_body.clone().unwrap_or_default(),
        timeout: None,
    }
}

fn caller_headers(call: &Call) -> Headers {
    call.extra_headers
        .iter()
        .flatten()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn chunk_size(call: &Call) -> Result<NonZeroUsize, SdkHttpError> {
    call.chunk_size.and_then(NonZeroUsize::new).ok_or_else(|| {
        SdkHttpError::Invalid(String::from(
            "a streaming content call needs a chunk_size of at least 1",
        ))
    })
}

fn buffered(content: BinaryContent) -> ContentValue {
    ContentValue::Buffered(BufferedContent {
        status: content.status,
        headers: text_headers(content.headers),
        body: content.body,
    })
}

fn text_headers(headers: IndexMap<String, String>) -> BTreeMap<String, Json> {
    headers
        .into_iter()
        .map(|(name, value)| (name, Json::Text(value)))
        .collect()
}

// The stream is drained to its end: LiteLLM hands the caller a finished iterator, and its exit is not part of the answer
async fn streamed<C, S>(
    prepared: PreparedContent<'_, C, S>,
    size: NonZeroUsize,
    state: &mut LoggingState,
) -> Result<Outcome, SdkHttpError>
where
    C: FileContentConfig,
    S: ClientSource<Client: ContentTransport>,
{
    let result = match retrieve_content_streaming(prepared, size, state).await {
        Ok(result) => result,
        Err(error) => return content_outcome(Err(error)),
    };
    let mut stream = result.stream;
    let mut chunks = Vec::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(bytes) => chunks.push(bytes),
            Err(FilesError::Body { source }) => {
                return Err(SdkHttpError::Stream(Box::new(source)));
            }
            Err(other) => return Err(SdkHttpError::Invalid(other.to_string())),
        }
    }
    let value = ContentValue::Streamed(StreamedContent {
        chunks,
        headers: text_headers(result.headers),
    });
    content_outcome(Ok(value))
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn content_outcome(result: Result<ContentValue, ContentError>) -> Result<Outcome, SdkHttpError> {
    match result {
        Ok(value) => Ok(Outcome {
            result: Some(value),
            failure: None,
        }),
        Err(ContentError::Provider(error)) => Ok(Outcome {
            result: None,
            failure: Some(Failure {
                status: error.status_code,
                synthesized: error.status_code_is_synthesized,
                message: error.message,
                headers: error.headers,
            }),
        }),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}
