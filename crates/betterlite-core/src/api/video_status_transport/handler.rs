use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::pre_call;
use betterlite_models::decisions::Json;

use super::channel::{AsynchronousKind, Channel, SynchronousKind};
use super::clients::{ClientFactories, Selected};
use super::config::{StatusConfig, VideoStatusConfig};
use super::error::{ErrorConverter, StatusFailure, VideoStatusError};
use super::logging::pre_call_record;
use super::records::{
    Headers, Params, PendingStatus, StatusCall, StatusMethod, StatusRequest, VideoEnvironment,
    VideoReply, VideoRequestContext, VideoStatusHandled,
};
use crate::api::video_records::VideoObject;

// The injected collaborators: the B173 client factories, the B249 logging environment and the B2402 error converter
pub struct VideoStatusHandler<'deps, Factories: ClientFactories> {
    pub factories: &'deps Factories,
    pub logging: &'deps PrePostEnv<'deps>,
    pub converter: &'deps dyn ErrorConverter,
}

impl<Factories: ClientFactories> VideoStatusHandler<'_, Factories> {
    // video_status_handler: the _is_async flag answers with the async call before any client is chosen
    pub fn video_status_handler<'call>(
        &'call self,
        config: &'call StatusConfig<'call, Factories>,
        call: StatusCall<'call, Factories::Synchronous, Factories::Asynchronous>,
        is_async: bool,
    ) -> Result<VideoStatusHandled<VideoObject, PendingStatus<'call>>, VideoStatusError> {
        if is_async {
            return Ok(VideoStatusHandled::Pending(Box::pin(
                self.async_video_status_handler(config, call),
            )));
        }
        match poll_once(self.run::<SynchronousKind>(config, call)) {
            Polled::Ready(outcome) => outcome.map(VideoStatusHandled::Ready),
            Polled::StillWaiting => Err(VideoStatusError::Suspended),
        }
    }

    // ast-grep-ignore: no-one-line-functions
    pub async fn async_video_status_handler<'call>(
        &'call self,
        config: &'call StatusConfig<'call, Factories>,
        call: StatusCall<'call, Factories::Synchronous, Factories::Asynchronous>,
    ) -> Result<VideoObject, VideoStatusError> {
        self.run::<AsynchronousKind>(config, call).await
    }

    // the one body both entry points run, with the try block being the send and the response transform
    async fn run<'call, Mode: Channel<Factories>>(
        &'call self,
        config: &'call StatusConfig<'call, Factories>,
        call: StatusCall<'call, Factories::Synchronous, Factories::Asynchronous>,
    ) -> Result<VideoObject, VideoStatusError> {
        let client = choose_client::<Factories, Mode>(self.factories, &call)?;
        let request = build_request(config, &call)?;
        let record = pre_call_record(&request, call.video_id);
        pre_call(call.logging, &record, self.logging).map(drop)?;
        let response = match Mode::send(client.get(), &request).await {
            Ok(response) => response,
            Err(error) => {
                let failure = StatusFailure::Send(&error);
                return Err(VideoStatusError::Converted(
                    self.converter.convert(config, failure),
                ));
            }
        };
        let reply = VideoReply {
            raw_response: &response,
            logging_obj: call.logging,
            custom_llm_provider: Some(call.custom_llm_provider),
        };
        match Mode::respond(config, reply, client.get()).await {
            Ok(video) => Ok(video),
            Err(failure) => {
                let caught = StatusFailure::Transform(&failure);
                Err(VideoStatusError::Converted(
                    self.converter.convert(config, caught),
                ))
            }
        }
    }
}

fn choose_client<'call, Factories: ClientFactories, Mode: Channel<Factories>>(
    factories: &Factories,
    call: &StatusCall<'call, Factories::Synchronous, Factories::Asynchronous>,
) -> Result<Selected<'call, Mode::Client>, VideoStatusError> {
    match Mode::supplied(call) {
        Some(supplied) => Ok(Selected::Supplied(supplied)),
        None => Ok(Selected::Created(Mode::build(
            factories,
            call.custom_llm_provider,
            call.litellm_params,
        )?)),
    }
}

// validate_environment, then extra_headers copied over it, then get_complete_url and the request transform
fn build_request<Synchronous, Asynchronous>(
    config: &dyn VideoStatusConfig<SyncClient = Synchronous, AsyncClient = Asynchronous>,
    call: &StatusCall<'_, Synchronous, Asynchronous>,
) -> Result<StatusRequest, VideoStatusError>
where
    Asynchronous: Sync,
{
    let empty = Headers::new();
    let given = call.extra_headers.unwrap_or(&empty);
    let mut headers = config.validate_environment(VideoEnvironment {
        headers: given,
        model: "",
        api_key: call.api_key,
        litellm_params: Some(call.litellm_params),
    })?;
    if let Some(extra) = call.extra_headers {
        headers.extend(extra.clone());
    }
    let base = api_base(call.litellm_params)?;
    let complete = config.get_complete_url("", base, call.litellm_params)?;
    let context = VideoRequestContext {
        api_base: &complete,
        litellm_params: call.litellm_params,
        headers: &headers,
    };
    let (url, data) = config.transform_video_status_retrieve_request(call.video_id, context)?;
    let (method, body) = if data.is_empty() {
        (StatusMethod::Get, None)
    } else {
        (StatusMethod::Post, Some(data))
    };
    Ok(StatusRequest {
        method,
        url,
        headers,
        body,
    })
}

// litellm_params api_base: absent or null means the provider's own default
fn api_base(litellm_params: &Params) -> Result<Option<&str>, VideoStatusError> {
    match litellm_params.get("api_base") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(base)) => Ok(Some(base)),
        Some(_) => Err(VideoStatusError::FieldType {
            field: "api_base",
            expected: "a string",
        }),
    }
}

enum Polled<Output> {
    Ready(Output),
    StillWaiting,
}

// the blocking path's single poll: its client answers at once, so only a broken one leaves this waiting
fn poll_once<Running: Future>(future: Running) -> Polled<Running::Output> {
    let mut pinned = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match pinned.as_mut().poll(&mut context) {
        Poll::Ready(outcome) => Polled::Ready(outcome),
        Poll::Pending => Polled::StillWaiting,
    }
}
