use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Extra;
use betterlite_openai::text_completion_transport::logging::StreamDefaults;
use indexmap::IndexMap;

use crate::azure_stream_errors::error::{Raised, StreamingError};
use crate::azure_stream_errors::records::{
    GetStreamClient, PreCallRecord, StreamCall, StreamClient, StreamHooks, StreamRuntime,
    StreamSource,
};
use crate::azure_stream_errors::stream::AzureChatStream;
use crate::azure_stream_errors::{
    async_streaming as run_async_streaming, streaming as run_streaming,
};

use super::super::arguments::{ChatCall, Runtime, SentChat};
use super::super::client::ClientSource;
use super::super::error::AzureChatError;
use super::super::hooks::ChatHooks;
use super::records;

// B685 resolves the stream client through its own source; completion has already chosen it
struct Delegated(StreamClient);

impl StreamSource for Delegated {
    fn get(&self, _request: GetStreamClient<'_>) -> Result<StreamClient, Box<Raised>> {
        Ok(self.0.clone())
    }
}

// B685's logging calls carry the same records, so the caller's hooks receive them unchanged
struct Adapted<'hooks> {
    hooks: &'hooks mut dyn ChatHooks,
}

impl StreamHooks for Adapted<'_> {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.hooks.pre_call(record)
    }

    fn response_headers(&mut self, headers: &IndexMap<String, String>) {
        self.hooks.response_headers(headers);
    }

    fn stream_defaults(&self) -> StreamDefaults {
        self.hooks.stream_defaults()
    }
}

// Completion.stream for either direction: B685 owns the stream bookkeeping
#[derive(Clone, Copy)]
pub enum Direction {
    Sync,
    Async,
}

pub async fn stream<Source>(
    source: &Source,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn ChatHooks,
    direction: Direction,
) -> Result<AzureChatStream, AzureChatError>
where
    Source: ClientSource,
{
    let requested = GetStreamClient {
        api_version: call.api_version,
        api_base: call.api_base,
        api_key: call.api_key,
        model: &sent.model,
        supplied: None,
        litellm_params: &sent.litellm_params,
        is_async: matches!(direction, Direction::Async),
    };
    let client = source
        .stream(sent.supplied.as_ref(), requested)
        .map_err(|error| AzureChatError::from(&error))?;
    let data = records::logged_object(&sent.data);
    let call_record = stream_call(sent, call, &client, &data);
    let delegated = Delegated(client);
    let mut adapted = Adapted { hooks };
    let outcome = match direction {
        Direction::Sync => {
            run_streaming(
                &delegated,
                call.sources,
                &call_record,
                &mut adapted,
                stream_runtime(runtime),
            )
            .await
        }
        Direction::Async => {
            run_async_streaming(
                &delegated,
                &call_record,
                &mut adapted,
                stream_runtime(runtime),
            )
            .await
        }
    };
    outcome.map_err(|error: StreamingError| AzureChatError::from_streaming(&error))
}

fn stream_call<'call, Client>(
    sent: &'call SentChat<Client>,
    call: &'call ChatCall<'_, Client>,
    supplied: &'call StreamClient,
    data: &'call Extra,
) -> StreamCall<'call> {
    StreamCall {
        api_base: call.api_base,
        api_key: call.api_key,
        api_version: call.api_version,
        dynamic_params: call.dynamic_params,
        data,
        model: &sent.model,
        timeout: Some(call.timeout),
        max_retries: sent.max_retries.count(call.azure.default_max_retries),
        azure_ad_token: call.azure_ad_token,
        azure_ad_token_provider: call.azure_ad_token_provider,
        litellm_params: &sent.litellm_params,
        supplied: Some(supplied),
    }
}

fn stream_runtime(runtime: &Runtime<'_>) -> StreamRuntime {
    StreamRuntime {
        now: runtime.stream_now,
        next_id: runtime.identifier,
    }
}
