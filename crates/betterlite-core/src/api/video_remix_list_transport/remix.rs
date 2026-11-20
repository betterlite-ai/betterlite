use std::future::Future;
use std::time::{Duration, Instant};

use betterlite_http::proxy::UpstreamResponse;

use crate::api::video_records::VideoObject;

use super::client::{Chosen, VideoClient, VideoClientSource, choose_async};
use super::config::VideoRemixListConfig;
use super::convert;
use super::error::{Operation, VideoRemixListTransportError};
use super::fields;
use super::logging::{PreCallSource, RemixPreCall, record_pre_call};
use super::records::{
    Headers, Params, VideoEnvironment, VideoLogging, VideoReply, VideoRequestContext,
};
use super::wire;
use super::{Handled, LogContext, RemixCall, VideoRemixListTransport};

struct PreparedRemix {
    url: String,
    headers: Headers,
    data: Params,
    timeout: Option<Duration>,
}

impl<Source: VideoClientSource> VideoRemixListTransport<Source> {
    // video_remix_handler: the _is_async hand-off comes before any client is picked
    pub fn remix<'call>(
        &self,
        config: &dyn VideoRemixListConfig,
        call: RemixCall<'call, Source>,
        mut log: LogContext<'_, '_>,
        is_async: bool,
    ) -> Result<
        Handled<
            VideoObject,
            impl Future<Output = Result<VideoObject, VideoRemixListTransportError>> + Send,
        >,
        VideoRemixListTransportError,
    > {
        if is_async {
            return Ok(Handled::Pending(self.async_remix(config, call, log)));
        }
        let chosen = match &call.client {
            Some(VideoClient::Sync(client)) => Chosen::Supplied(*client),
            Some(VideoClient::Asynchronous(_)) | None => {
                let ssl_verify = fields::ssl_verify(call.litellm_params)?;
                let client = self.source.sync_client(ssl_verify).map_err(|source| {
                    VideoRemixListTransportError::Client {
                        source: Box::new(source),
                    }
                })?;
                Chosen::Pooled(client)
            }
        };
        let prepared = prepare_remix(config, &call, &mut log)?;
        let answer = send_remix(config, chosen.get(), &prepared)?;
        let object = finish_remix(config, &call, &answer, &mut log)?;
        Ok(Handled::Done(object))
    }

    // async_video_remix_handler: the same steps on the pooled async client
    pub async fn async_remix(
        &self,
        config: &dyn VideoRemixListConfig,
        call: RemixCall<'_, Source>,
        mut log: LogContext<'_, '_>,
    ) -> Result<VideoObject, VideoRemixListTransportError> {
        let supplied = match &call.client {
            Some(VideoClient::Asynchronous(client)) => Some(*client),
            Some(VideoClient::Sync(_)) | None => None,
        };
        let chosen = choose_async(
            &self.source,
            supplied,
            call.custom_llm_provider,
            call.litellm_params,
        )?;
        let prepared = prepare_remix(config, &call, &mut log)?;
        let answer = send_remix_async(config, chosen.get(), &prepared).await?;
        let checked = convert::check_status(config, answer)?;
        finish_remix(config, &call, &checked, &mut log)
    }
}

// validate_environment gets the extra headers and the argument-or-params key; the caller's headers are re-applied over its answer
fn remix_headers<Source: VideoClientSource>(
    config: &dyn VideoRemixListConfig,
    call: &RemixCall<'_, Source>,
) -> Result<Headers, VideoRemixListTransportError> {
    let key = match call.api_key {
        Some(key) => Some(key.clone()),
        None => fields::api_key(call.litellm_params)?,
    };
    let empty = Headers::new();
    let extra = call.extra_headers.unwrap_or(&empty);
    let mut headers = config.validate_environment(VideoEnvironment {
        headers: extra,
        model: "",
        api_key: key.as_ref(),
        litellm_params: Some(call.litellm_params),
    });
    if let Some(extra_headers) = call.extra_headers {
        for (name, value) in extra_headers {
            headers.insert(name.clone(), value.clone());
        }
    }
    Ok(headers)
}

// The provider's request transform turns the base URL and the validated headers into the wire URL and body
fn remix_request<Source: VideoClientSource>(
    config: &dyn VideoRemixListConfig,
    call: &RemixCall<'_, Source>,
    base_url: &str,
    headers: &Headers,
) -> Result<(String, Params), VideoRemixListTransportError> {
    config
        .transform_video_remix_request(
            call.video_id,
            call.prompt,
            call.extra_body,
            VideoRequestContext {
                api_base: base_url,
                litellm_params: call.litellm_params,
                headers,
            },
        )
        .map_err(|source| VideoRemixListTransportError::Prepare {
            operation: Operation::Remix,
            source,
        })
}

// The try block's send on the async path: one POST, with the caller's timeout or the client's own
async fn send_remix_async<Source: VideoClientSource>(
    config: &dyn VideoRemixListConfig,
    client: &Source::AsyncClient,
    prepared: &PreparedRemix,
) -> Result<UpstreamResponse, VideoRemixListTransportError> {
    let request = wire::json_post(
        &prepared.url,
        &prepared.headers,
        &prepared.data,
        prepared.timeout,
    )
    .map_err(|error| convert::wire_error(config, &error))?;
    let timeout = prepared.timeout.unwrap_or(client.default_timeout());
    let started = Instant::now();
    let sent = client.send(request).await;
    let elapsed = started.elapsed();
    sent.map_err(|error| {
        convert::transport_error(
            config,
            &convert::SendFacts {
                operation: Operation::Remix,
                mode: convert::SendMode::Asynchronous,
                timeout: Some(timeout),
                elapsed,
            },
            &error,
        )
    })
}

// Everything before the try block: the environment, the base URL, the request transform and the pre-call
fn prepare_remix<Source: VideoClientSource>(
    config: &dyn VideoRemixListConfig,
    call: &RemixCall<'_, Source>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedRemix, VideoRemixListTransportError> {
    let headers = remix_headers(config, call)?;
    let api_base = fields::api_base(call.litellm_params)?;
    let base_url = config
        .get_complete_url("", api_base, call.litellm_params)
        .map_err(|source| VideoRemixListTransportError::Prepare {
            operation: Operation::Remix,
            source,
        })?;
    let (url, data) = remix_request(config, call, &base_url, &headers)?;
    record_pre_call(
        log,
        PreCallSource::Remix(&RemixPreCall {
            prompt: call.prompt,
            url: &url,
            headers: &headers,
            data: &data,
            video_id: call.video_id,
        }),
    )?;
    Ok(PreparedRemix {
        url,
        headers,
        data,
        timeout: call.timeout,
    })
}

// The try block's send: the wire form, one POST with the timeout, and the status check before the response transform
fn send_remix<Source: VideoClientSource>(
    config: &dyn VideoRemixListConfig,
    client: &Source::SyncClient,
    prepared: &PreparedRemix,
) -> Result<UpstreamResponse, VideoRemixListTransportError> {
    let request = wire::json_post(
        &prepared.url,
        &prepared.headers,
        &prepared.data,
        prepared.timeout,
    )
    .map_err(|error| convert::wire_error(config, &error))?;
    let started = Instant::now();
    let sent = client.send(request);
    let elapsed = started.elapsed();
    let answer = sent.map_err(|error| {
        convert::transport_error(
            config,
            &convert::SendFacts {
                operation: Operation::Remix,
                mode: convert::SendMode::Sync,
                timeout: prepared.timeout,
                elapsed,
            },
            &error,
        )
    })?;
    convert::check_status(config, answer)
}

// The response transform runs against the real logging details, and whatever it stores there is kept
fn finish_remix<Source: VideoClientSource>(
    config: &dyn VideoRemixListConfig,
    call: &RemixCall<'_, Source>,
    answer: &UpstreamResponse,
    log: &mut LogContext<'_, '_>,
) -> Result<VideoObject, VideoRemixListTransportError> {
    let mut logging = VideoLogging {
        model_call_details: log.state.model_call_details.clone(),
    };
    let outcome = config.transform_video_remix_response(VideoReply {
        raw_response: answer,
        logging_obj: &mut logging,
        custom_llm_provider: Some(call.custom_llm_provider),
    });
    log.state.model_call_details = logging.model_call_details;
    outcome.map_err(|_failure| convert::status_error(config, answer))
}
