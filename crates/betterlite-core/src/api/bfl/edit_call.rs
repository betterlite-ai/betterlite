pub mod error;
pub mod prepare;

mod completed;
mod tests;

use std::str;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use http::header::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use url::Url;

use self::completed::{Completed, Recording};
use self::error::EditCallError;
use self::prepare::{PreparedEdit, insert_header, prepare};
use crate::api::bfl::edit::media::{MaskInput, SuppliedMedia};
use crate::api::bfl::generation::BflImageResponse;
use crate::api::bfl::generation_call::LogContext;
use crate::api::bfl::generation_call::params::CallParams;
use crate::api::bfl::generation_call::submit::{SubmitError, SubmitRequest, SubmitTransport};
use crate::api::bfl::poll::{BflPoller, PollClock, PollSleeper, PollTransport};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

pub struct EditCall<'text> {
    pub model: &'text str,
    pub prompt: Option<&'text str>,
    pub image: Option<&'text mut SuppliedMedia>,
    pub mask: MaskInput<'text>,
    pub optional_params: &'text ImageParams,
    pub params: CallParams,
    pub extra_headers: &'text [(String, String)],
    pub timeout: Option<Duration>,
    pub aimage_edit: bool,
    pub max_depth: i64,
    pub configured_key: Option<&'text SecretString>,
    pub configured_base: &'text str,
    pub now_seconds: i64,
}

pub struct EditExecutor<'deps, P, C, S> {
    pub provider: &'deps P,
    pub clock: &'deps C,
    pub sleeper: &'deps S,
}

impl<P, C, S> EditExecutor<'_, P, C, S>
where
    P: SubmitTransport + PollTransport + Sync,
    C: PollClock + Sync,
    S: PollSleeper + Sync,
{
    // LiteLLM's sync and async edit entry points differ only in which client they keep
    pub async fn image_edit<T>(
        &self,
        call: &EditCall<'_>,
        prepared: &PreparedEdit,
        supplied: Option<SuppliedClient<'_, T>>,
    ) -> Result<BflImageResponse, EditCallError>
    where
        T: SubmitTransport + PollTransport + Sync,
    {
        let expected = if call.aimage_edit {
            ClientKind::Asynchronous
        } else {
            ClientKind::Sync
        };
        match supplied.filter(|client| client.kind == expected) {
            Some(client) => self.execute(call, prepared, client.transport).await,
            None => self.execute(call, prepared, self.provider).await,
        }
    }

    async fn execute<T>(
        &self,
        call: &EditCall<'_>,
        prepared: &PreparedEdit,
        transport: &T,
    ) -> Result<BflImageResponse, EditCallError>
    where
        T: SubmitTransport + PollTransport + Sync,
    {
        let (response, headers) = submit(call, prepared, transport).await?;
        let completed = Completed::default();
        let recorded = Recording {
            transport,
            completed: &completed,
        };
        let poller = BflPoller {
            transport: &recorded,
            clock: self.clock,
            sleeper: self.sleeper,
        };
        // the shared poller owns the body, polling-url and errors-key checks the edit handler repeats
        poller.poll(&response, &headers).await?;
        completed.project(call.now_seconds)
    }
}

// LiteLLM applies both header layers to one dict before the POST, so only the client refuses a header it cannot encode
async fn submit<T>(
    call: &EditCall<'_>,
    prepared: &PreparedEdit,
    transport: &T,
) -> Result<(UpstreamResponse, HeaderMap), EditCallError>
where
    T: SubmitTransport + Sync,
{
    let mut headers = prepared.headers.clone();
    for (name, value) in call.extra_headers {
        insert_header(&mut headers, name, value)?;
    }
    let url = Url::parse(&prepared.url).map_err(|source| EditCallError::Submission {
        source: SubmitError::Url(source),
    })?;
    let request = SubmitRequest {
        url,
        headers: headers.clone(),
        body: Json::Object(prepared.body.clone()),
        timeout: call.timeout,
    };
    let response = transport
        .post(request)
        .await
        .map_err(|source| EditCallError::Submission { source })?;
    // httpx raises on any status outside 2xx, so LiteLLM turns a rejection or an unfollowed redirect into the same 500 as a failed request
    if !(200..=299).contains(&response.status) {
        return Err(EditCallError::SubmissionStatus {
            status: response.status,
        });
    }
    Ok((response, headers))
}

// LiteLLM records the pre-call before the POST
pub fn prepare_call(
    call: &mut EditCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedEdit, EditCallError> {
    let prepared = prepare(call)?;
    log_pre_call(call, &prepared, log)?;
    Ok(prepared)
}

fn log_pre_call(
    call: &EditCall<'_>,
    prepared: &PreparedEdit,
    log: &mut LogContext<'_, '_>,
) -> Result<(), EditCallError> {
    let mut headers = IndexMap::new();
    for (name, value) in &prepared.headers {
        let text = str::from_utf8(value.as_bytes()).map_or_else(
            |_| format!("{} bytes that are not UTF-8", value.as_bytes().len()),
            str::to_owned,
        );
        headers.insert(name.as_str().to_owned(), Json::Text(text));
    }
    // the pre-call record carries the caller's headers as given, before the wire refuses any of them
    for (name, value) in call.extra_headers {
        headers.insert(name.to_ascii_lowercase(), Json::Text(value.clone()));
    }
    let record = PreCall {
        input: call
            .prompt
            .map_or(Json::Null, |prompt| Json::Text(prompt.to_owned())),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(prepared.url.clone()),
            headers: Some(headers),
            complete_input_dict: Some(RawBody::Object(prepared.body.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)?;
    Ok(())
}
