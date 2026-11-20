use std::future::Future;
use std::time::{Duration, Instant};

use betterlite_http::proxy::UpstreamResponse;
use secrecy::SecretString;

use crate::api::video_records::VideoObject;

use super::client::{
    Chosen, VideoAsyncClient as _, VideoClient, VideoClientSource, VideoRequest,
    VideoSyncClient as _, choose_async,
};
use super::config::VideoRemixListConfig;
use super::convert;
use super::error::{Operation, VideoRemixListTransportError};
use super::fields;
use super::logging::{PreCallSource, RemixPreCall, record_pre_call};
use super::records::{Headers, Params, VideoEnvironment, VideoRequestContext};
use super::response::{RemixResponse, transform_response};
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
    pub fn remix(
        &self,
        config: &dyn VideoRemixListConfig,
        call: RemixCall<'_, Source>,
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
                let ssl_verify = fields::read::<bool>(call.litellm_params, "ssl_verify")?;
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
        let object = transform_response::<RemixResponse>(
            config,
            &answer,
            call.custom_llm_provider,
            &mut log,
        )?;
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
        let request = remix_wire_request(config, &prepared)?;
        let timeout = prepared.timeout.unwrap_or(chosen.get().default_timeout());
        let started = Instant::now();
        let sent = chosen.get().send(request).await;
        let facts = convert::SendFacts {
            operation: Operation::Remix,
            mode: convert::SendMode::Asynchronous,
            timeout: Some(timeout),
            elapsed: started.elapsed(),
        };
        let answer = sent
            .map_err(|error| convert::failure(config, &convert::transport_facts(&facts, &error)))?;
        let checked = convert::check_status(config, answer)?;
        transform_response::<RemixResponse>(config, &checked, call.custom_llm_provider, &mut log)
    }
}

// validate_environment gets the extra headers and the argument-or-params key; the caller's headers are re-applied over its answer
fn remix_headers<Source>(
    config: &dyn VideoRemixListConfig,
    call: &RemixCall<'_, Source>,
) -> Result<Headers, VideoRemixListTransportError>
where
    Source: VideoClientSource,
{
    let key = match call.api_key {
        Some(key) => Some(key.clone()),
        None => fields::read::<SecretString>(call.litellm_params, "api_key")?,
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
fn remix_request<Source>(
    config: &dyn VideoRemixListConfig,
    call: &RemixCall<'_, Source>,
    base_url: &str,
    headers: &Headers,
) -> Result<(String, Params), VideoRemixListTransportError>
where
    Source: VideoClientSource,
{
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

// The try block's POST, with the caller's timeout; a call without one leaves the client's own in place
fn remix_wire_request(
    config: &dyn VideoRemixListConfig,
    prepared: &PreparedRemix,
) -> Result<VideoRequest, VideoRemixListTransportError> {
    wire::json_post(
        &prepared.url,
        &prepared.headers,
        &prepared.data,
        prepared.timeout,
    )
    .map_err(|error| convert::failure(config, &convert::display_facts(error.to_string())))
}

// Everything before the try block: the environment, the base URL, the request transform and the pre-call
fn prepare_remix<Source>(
    config: &dyn VideoRemixListConfig,
    call: &RemixCall<'_, Source>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedRemix, VideoRemixListTransportError>
where
    Source: VideoClientSource,
{
    let headers = remix_headers(config, call)?;
    let api_base = fields::read::<String>(call.litellm_params, "api_base")?;
    let base_url = config
        .get_complete_url("", api_base.as_deref(), call.litellm_params)
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
fn send_remix<Source>(
    config: &dyn VideoRemixListConfig,
    client: &Source::SyncClient,
    prepared: &PreparedRemix,
) -> Result<UpstreamResponse, VideoRemixListTransportError>
where
    Source: VideoClientSource,
{
    let request = remix_wire_request(config, prepared)?;
    let started = Instant::now();
    let sent = client.send(request);
    let facts = convert::SendFacts {
        operation: Operation::Remix,
        mode: convert::SendMode::Sync,
        timeout: prepared.timeout,
        elapsed: started.elapsed(),
    };
    let answer =
        sent.map_err(|error| convert::failure(config, &convert::transport_facts(&facts, &error)))?;
    convert::check_status(config, answer)
}
