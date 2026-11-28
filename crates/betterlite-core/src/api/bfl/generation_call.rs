pub mod error;
pub mod params;
pub mod prepare;
pub mod submit;

mod tests;

use std::str;
use std::time::Duration;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::reply::ImageDatum;
use http::header::HeaderValue;
use indexmap::IndexMap;
use secrecy::SecretString;
use url::Url;

use self::error::GenerationCallError;
use self::params::CallParams;
use self::prepare::{PreparedGeneration, overlay_headers, prepare};
use self::submit::{SubmitError, SubmitRequest, SubmitTransport};
use crate::api::bfl::generation::{BflImageResponse, apply_image_response};
use crate::api::bfl::poll::{BflPoller, PollClock, PollSleeper, PollTransport};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

pub struct GenerationCall<'text> {
    pub model: &'text str,
    pub prompt: &'text str,
    pub optional_params: &'text IndexMap<String, Json>,
    pub params: CallParams,
    pub extra_headers: &'text [(String, String)],
    pub timeout: Option<Duration>,
    pub aimg_generation: bool,
    pub configured_key: Option<&'text SecretString>,
    pub configured_base: &'text str,
    pub now_seconds: i64,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub struct GenerationExecutor<'deps, P, C, S> {
    pub provider: &'deps P,
    pub clock: &'deps C,
    pub sleeper: &'deps S,
}

impl<P, C, S> GenerationExecutor<'_, P, C, S>
where
    P: SubmitTransport + PollTransport + Sync,
    C: PollClock + Sync,
    S: PollSleeper + Sync,
{
    // LiteLLM's sync and async entry points differ only in which client they pick, so both run this one operation
    pub async fn image_generation<T>(
        &self,
        call: &GenerationCall<'_>,
        prepared: &PreparedGeneration,
        supplied: Option<SuppliedClient<'_, T>>,
        data: Vec<ImageDatum>,
    ) -> Result<BflImageResponse, GenerationCallError>
    where
        T: SubmitTransport + PollTransport + Sync,
    {
        let expected = if call.aimg_generation {
            ClientKind::Asynchronous
        } else {
            ClientKind::Sync
        };
        match supplied.filter(|client| client.kind == expected) {
            Some(client) => self.execute(call, prepared, client.transport, data).await,
            None => self.execute(call, prepared, self.provider, data).await,
        }
    }

    async fn execute<T>(
        &self,
        call: &GenerationCall<'_>,
        prepared: &PreparedGeneration,
        transport: &T,
        data: Vec<ImageDatum>,
    ) -> Result<BflImageResponse, GenerationCallError>
    where
        T: SubmitTransport + PollTransport + Sync,
    {
        let headers = overlay_headers(&prepared.headers, call.extra_headers)?;
        let url = Url::parse(&prepared.url).map_err(|source| GenerationCallError::Submission {
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
            .map_err(|source| GenerationCallError::Submission { source })?;
        // httpx raises on any status outside 2xx, so LiteLLM turns a rejection or an unfollowed redirect into the same 500 as a failed request
        if !(200..=299).contains(&response.status) {
            return Err(GenerationCallError::SubmissionStatus {
                status: response.status,
            });
        }
        let polled = BflPoller {
            transport,
            clock: self.clock,
            sleeper: self.sleeper,
        }
        .poll(&response, &headers)
        .await?;
        let result = apply_image_response(&polled, data, call.now_seconds)?;
        Ok(result)
    }
}

// LiteLLM records the pre-call before the POST
pub fn prepare_call(
    call: &GenerationCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedGeneration, GenerationCallError> {
    let prepared = prepare(call)?;
    log_pre_call(call, &prepared, log)?;
    Ok(prepared)
}

fn log_pre_call(
    call: &GenerationCall<'_>,
    prepared: &PreparedGeneration,
    log: &mut LogContext<'_, '_>,
) -> Result<(), GenerationCallError> {
    let mut headers: IndexMap<String, Json> = prepared
        .headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), Json::Text(header_text(value))))
        .collect();
    // the pre-call record carries the caller's headers as given, before the wire refuses any of them
    for (name, value) in call.extra_headers {
        headers.insert(name.to_ascii_lowercase(), Json::Text(value.clone()));
    }
    let record = PreCall {
        input: Json::Text(call.prompt.to_owned()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(prepared.url.clone()),
            headers: Some(headers),
            complete_input_dict: Some(RawBody::Object(prepared.body.clone())),
            request_str: None,
            openai_sdk: None,
            // B249 keeps no api_key, so LiteLLM's empty one has no field here
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)?;
    Ok(())
}

fn header_text(value: &HeaderValue) -> String {
    str::from_utf8(value.as_bytes()).map_or_else(
        |_| format!("{} bytes that are not UTF-8", value.as_bytes().len()),
        ToOwned::to_owned,
    )
}
