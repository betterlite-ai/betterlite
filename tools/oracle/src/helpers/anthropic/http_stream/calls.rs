use std::mem;
use std::sync::PoisonError;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_anthropic::anthropic_http_stream::call::{
    OpenedStream, StreamBuild, StreamCall, StreamClients, StreamOpening,
};
use betterlite_anthropic::anthropic_http_stream::error::{PostFailure, StreamOpenError};
use betterlite_anthropic::anthropic_http_stream::transport::StreamPost;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use tokio::runtime::Builder;

use super::fixture::{
    FixedCounter, FixedIds, FixtureTransport, HelperFailure, SilentLogger, Verdict,
};
use super::support::header_map;
use super::{Input, MAX_EVENT_BYTES};

pub(crate) const TIMEOUT: Duration = Duration::from_millis(1500);

pub(crate) type Attempted =
    Result<OpenedStream<HelperFailure, FixedCounter>, StreamOpenError<HelperFailure>>;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Outcome {
    Reply,
    Status,
    Unknown,
    Timeout,
}

pub(crate) struct Attempt {
    pub(crate) result: Attempted,
    pub(crate) posts: Vec<StreamPost>,
    pub(crate) polled: bool,
}

pub(crate) fn transport(
    input: &Input,
    outcome: Outcome,
) -> Result<FixtureTransport, Box<HelperError>> {
    let headers = header_map(&input.upstream_headers)?;
    match outcome {
        Outcome::Reply => Ok(FixtureTransport::reply(
            input.upstream_status.unwrap_or(200),
            headers,
            &input.upstream_lines,
        )),
        Outcome::Status => Ok(FixtureTransport::status(
            input.upstream_status.unwrap_or(400),
            input.upstream_body.as_deref().unwrap_or_default(),
            headers,
        )),
        Outcome::Unknown => Ok(FixtureTransport::new(Verdict::Failed(
            PostFailure::Unknown(HelperFailure::Unknown {
                message: input
                    .failure_message
                    .as_deref()
                    .unwrap_or("the transport exploded")
                    .to_owned(),
            }),
        ))),
        Outcome::Timeout => Ok(FixtureTransport::new(Verdict::Failed(PostFailure::Known(
            HelperFailure::Timeout,
        )))),
    }
}

pub(crate) fn attempt(
    input: &Input,
    synchronous: bool,
    outcome: Outcome,
) -> Result<Attempt, Box<HelperError>> {
    let client = transport(input, outcome)?;
    let logger = SilentLogger;
    let clients = StreamClients {
        supplied: Some(&client),
        module_level_aclient: &client,
        module_level_client: &client,
        logger: &logger,
    };
    let call = StreamCall {
        api_base: &input.api_base,
        headers: header_map(&input.headers)?,
        data: &input.data,
        messages: &input.messages,
        timeout: TIMEOUT,
        json_mode: input.json_mode,
        speed: input.speed.as_deref(),
        tool_name_reverse_map: &input.reverse_map,
    };
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let build = StreamBuild {
        ids: &FixedIds,
        counter: FixedCounter,
        max_event_bytes: MAX_EVENT_BYTES,
        max_error_body_bytes: MAX_EVENT_BYTES,
    };
    let result = runtime.block_on(async move {
        if synchronous {
            StreamOpening {
                clients,
                call: &call,
                build,
            }
            .make_sync_call()
            .await
        } else {
            StreamOpening {
                clients,
                call: &call,
                build,
            }
            .make_call()
            .await
        }
    });
    Ok(Attempt {
        result,
        posts: mem::take(&mut *client.posts.lock().unwrap_or_else(PoisonError::into_inner)),
        polled: client.polled.load(Ordering::SeqCst),
    })
}
