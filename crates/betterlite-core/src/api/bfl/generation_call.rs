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
use self::prepare::{PreparedGeneration, prepare};
use self::submit::{SubmitError, SubmitRequest, SubmitTransport, SuppliedTransport, TransportKind};
use crate::api::bfl::generation::{BflImageResponse, apply_image_response};
use crate::api::bfl::poll::{BflPoller, PollClock, PollSleeper, PollTransport};

pub struct GenerationCall<'text> {
    pub model: &'text str,
    pub prompt: &'text str,
    pub optional_params: &'text IndexMap<String, Json>,
    pub params: CallParams,
    pub extra_headers: Option<&'text [(String, String)]>,
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
        supplied: Option<SuppliedTransport<'_, T>>,
        data: Vec<ImageDatum>,
        log: &mut LogContext<'_, '_>,
    ) -> Result<BflImageResponse, GenerationCallError>
    where
        T: SubmitTransport + PollTransport + Sync,
    {
        let prepared = prepare(call)?;
        log_pre_call(call, &prepared, log)?;
        let expected = if call.aimg_generation {
            TransportKind::Async
        } else {
            TransportKind::Sync
        };
        match supplied.filter(|client| client.kind == expected) {
            Some(client) => self.execute(call, &prepared, client.transport, data).await,
            None => self.execute(call, &prepared, self.provider, data).await,
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
        let url = Url::parse(&prepared.url).map_err(|source| GenerationCallError::Submission {
            source: SubmitError::Url(source),
        })?;
        let request = SubmitRequest {
            url,
            headers: prepared.headers.clone(),
            body: Json::Object(prepared.body.clone()),
            timeout: call.timeout,
        };
        let response = transport
            .post(request)
            .await
            .map_err(|source| GenerationCallError::Submission { source })?;
        // a rejected submission reaches LiteLLM as a raised exception, so it becomes the same 500 as a failed request
        if response.status >= 400 {
            return Err(GenerationCallError::SubmissionStatus {
                status: response.status,
            });
        }
        let polled = BflPoller {
            transport,
            clock: self.clock,
            sleeper: self.sleeper,
        }
        .poll(&response, &prepared.headers)
        .await?;
        let result = apply_image_response(&polled, data, call.now_seconds)?;
        Ok(result)
    }
}

fn log_pre_call(
    call: &GenerationCall<'_>,
    prepared: &PreparedGeneration,
    log: &mut LogContext<'_, '_>,
) -> Result<(), GenerationCallError> {
    let headers: IndexMap<String, Json> = prepared
        .headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), Json::Text(header_text(value))))
        .collect();
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
        str::to_owned,
    )
}
