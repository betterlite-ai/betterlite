use std::future::Future;
use std::str::FromStr as _;
use std::time::Instant;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::identity::provider_id::ProviderId;
use tokio::runtime::{Builder, Handle};

use super::client::{Chosen, VideoClient, VideoClientSource};
use super::config::VideoRemixListConfig;
use super::convert;
use super::error::{Operation, VideoRemixListTransportError};
use super::fields;
use super::logging::{ListPreCall, PreCallSource, record_pre_call};
use super::records::{
    Headers, Params, VideoEnvironment, VideoListQuery, VideoLogging, VideoReply,
    VideoRequestContext,
};
use super::wire;
use super::{Handled, ListCall, LogContext, VideoRemixListTransport};

struct PreparedList {
    url: String,
    headers: Headers,
    params: Params,
}

impl<Source: VideoClientSource> VideoRemixListTransport<Source> {
    // video_list_handler: the _is_async hand-off, or the async handler run to completion with no api_key
    pub fn list<'call>(
        &self,
        config: &dyn VideoRemixListConfig,
        call: ListCall<'call, Source>,
        log: LogContext<'_, '_>,
        is_async: bool,
    ) -> Result<
        Handled<Params, impl Future<Output = Result<Params, VideoRemixListTransportError>> + Send>,
        VideoRemixListTransportError,
    > {
        if is_async {
            return Ok(Handled::Pending(self.async_list(config, call, log)));
        }
        if Handle::try_current().is_ok() {
            return Err(VideoRemixListTransportError::RunningRuntime { status_code: 500 });
        }
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| VideoRemixListTransportError::Runtime { source })?;
        let async_call = ListCall {
            api_key: None,
            ..call
        };
        runtime
            .block_on(self.async_list(config, async_call, log))
            .map(Handled::Done)
    }

    // async_video_list_handler: validate_environment sees only the api_key argument, never the params' one
    pub async fn async_list(
        &self,
        config: &dyn VideoRemixListConfig,
        call: ListCall<'_, Source>,
        mut log: LogContext<'_, '_>,
    ) -> Result<Params, VideoRemixListTransportError> {
        let chosen = match &call.client {
            Some(VideoClient::Asynchronous(client)) => Chosen::Supplied(*client),
            Some(VideoClient::Sync(_)) | None => {
                let provider =
                    ProviderId::from_str(call.custom_llm_provider).map_err(|_unknown| {
                        VideoRemixListTransportError::UnknownProvider {
                            provider: call.custom_llm_provider.to_owned(),
                        }
                    })?;
                let ssl_verify = fields::ssl_verify(call.litellm_params)?;
                let client = self
                    .source
                    .async_client(provider, ssl_verify)
                    .map_err(|source| VideoRemixListTransportError::Client {
                        source: Box::new(source),
                    })?;
                Chosen::Pooled(client)
            }
        };
        let prepared = prepare_list(config, &call, &mut log)?;
        let request = wire::query_get(&prepared.url, &prepared.headers, &prepared.params)
            .map_err(|error| convert::wire_error(config, &error))?;
        let started = Instant::now();
        let sent = chosen.get().send(request).await;
        let elapsed = started.elapsed();
        let answer = sent.map_err(|error| {
            convert::transport_error(
                config,
                &convert::SendFacts {
                    operation: Operation::List,
                    mode: convert::SendMode::Asynchronous,
                    timeout: call.timeout,
                    elapsed,
                },
                &error,
            )
        })?;
        let checked = convert::check_status(config, answer)?;
        finish_list(config, &call, &checked, &mut log)
    }
}

// Everything before the try block: the environment (api_key only), the base URL, the query transform and the pre-call
fn prepare_list<Source: VideoClientSource>(
    config: &dyn VideoRemixListConfig,
    call: &ListCall<'_, Source>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedList, VideoRemixListTransportError> {
    let empty = Headers::new();
    let extra = call.extra_headers.unwrap_or(&empty);
    let mut headers = config.validate_environment(VideoEnvironment {
        headers: extra,
        model: "",
        api_key: call.api_key,
        litellm_params: Some(call.litellm_params),
    });
    if let Some(extra_headers) = call.extra_headers {
        for (name, value) in extra_headers {
            headers.insert(name.clone(), value.clone());
        }
    }
    let api_base = fields::api_base(call.litellm_params)?;
    let base_url = config
        .get_complete_url("", api_base, call.litellm_params)
        .map_err(|source| VideoRemixListTransportError::Prepare {
            operation: Operation::List,
            source,
        })?;
    let (url, params) = config
        .transform_video_list_request(
            VideoListQuery {
                after: call.after,
                limit: call.limit,
                order: call.order,
                extra_query: call.extra_query,
            },
            VideoRequestContext {
                api_base: &base_url,
                litellm_params: call.litellm_params,
                headers: &headers,
            },
        )
        .map_err(|source| VideoRemixListTransportError::Prepare {
            operation: Operation::List,
            source,
        })?;
    record_pre_call(
        log,
        PreCallSource::List(&ListPreCall {
            url: &url,
            headers: &headers,
            params: &params,
        }),
    )?;
    Ok(PreparedList {
        url,
        headers,
        params,
    })
}

// The response transform runs against the real logging details, and whatever it stores there is kept
fn finish_list<Source: VideoClientSource>(
    config: &dyn VideoRemixListConfig,
    call: &ListCall<'_, Source>,
    answer: &UpstreamResponse,
    log: &mut LogContext<'_, '_>,
) -> Result<Params, VideoRemixListTransportError> {
    let mut logging = VideoLogging {
        model_call_details: log.state.model_call_details.clone(),
    };
    let outcome = config.transform_video_list_response(VideoReply {
        raw_response: answer,
        logging_obj: &mut logging,
        custom_llm_provider: Some(call.custom_llm_provider),
    });
    log.state.model_call_details = logging.model_call_details;
    outcome.map_err(|_failure| convert::status_error(config, answer))
}
