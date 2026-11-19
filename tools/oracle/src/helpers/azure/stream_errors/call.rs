use std::future::Future;

use betterlite_azure::azure_stream_errors::error::StreamingError;
use betterlite_azure::azure_stream_errors::records::{StreamCall, StreamRuntime};
use betterlite_azure::azure_stream_errors::{async_streaming, streaming};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use futures_util::StreamExt as _;
use secrecy::SecretString;
use tokio::runtime::Builder;
use uuid::Uuid;

use super::hooks::Recorder;
use super::source::{ScriptedSource, ScriptedSources, callable_provider};
use super::view;
use super::{Function, Input, StreamCase};

const CLOCK: i64 = 1_700_000_000;
const STREAM_TIMESTAMP: f64 = 1_700_000_000.0;

fn now() -> StreamInstant {
    StreamInstant {
        seconds: CLOCK,
        timestamp: STREAM_TIMESTAMP,
    }
}

fn block_on<Fut: Future>(future: Fut) -> Result<Fut::Output, Box<HelperError>> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    Ok(runtime.block_on(future))
}

fn one(function: &Function, case: &StreamCase) -> Result<Json, Box<HelperError>> {
    let source = ScriptedSource::new(case);
    let sources = ScriptedSources::new(&case.exchange);
    let mut recorder = Recorder::default();
    let client = source.client.clone();
    let key = case.api_key.clone().map(SecretString::from);
    let token = case.azure_ad_token.clone().map(SecretString::from);
    let provider = case.azure_ad_token_provider.map(callable_provider);
    let litellm_params = case.litellm_params.clone();
    let call = StreamCall {
        api_base: &case.api_base,
        api_key: key.as_ref(),
        api_version: &case.api_version,
        data: &case.data,
        model: &case.model,
        timeout: case.timeout,
        max_retries: case.max_retries,
        azure_ad_token: token.as_ref(),
        azure_ad_token_provider: provider.as_ref(),
        litellm_params: &litellm_params,
        supplied: Some(&client),
    };
    let _dynamic = case.dynamic_params;
    let runtime = StreamRuntime {
        now,
        next_id: || Uuid::from_u128(11),
    };
    let outcome = block_on(async {
        match function {
            Function::Streaming => {
                streaming(&source, &sources, &call, &mut recorder, runtime).await
            }
            Function::AsyncStreaming => {
                async_streaming(&source, &call, &mut recorder, runtime).await
            }
        }
    })?;
    match outcome {
        Ok(stream) => {
            let wrapper = view::wrapper_value(&stream);
            let chunks = block_on(async move {
                let mut items = stream.chunks;
                let mut values = Vec::new();
                while let Some(item) = items.next().await {
                    match item {
                        Ok(chunk) => values.push(view::chunk_value(&chunk)),
                        Err(error) => return Err(view::failure(&error)),
                    }
                }
                Ok(values)
            })??;
            Ok(view::report(
                &source,
                &sources,
                &recorder,
                "stream",
                wrapper,
                Json::List(chunks),
                Json::Null,
            ))
        }
        Err(error) => match error {
            StreamingError::Azure(failure) => Ok(view::report(
                &source,
                &sources,
                &recorder,
                "error",
                Json::Null,
                Json::Null,
                view::error_value(&failure),
            )),
            other => Err(view::failure(&other)),
        },
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let values = input
        .cases
        .iter()
        .map(|case| one(&input.function, case))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(view::calls(values))
}
