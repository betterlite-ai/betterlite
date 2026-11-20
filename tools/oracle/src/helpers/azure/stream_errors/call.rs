use std::future::Future;

use betterlite_azure::azure_stream_errors::records::{StreamCall, StreamRuntime};
use betterlite_azure::azure_stream_errors::stream::AzureChatStream;
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
use super::source::ScriptedSource;
use super::source::credentials::{ScriptedSources, callable_provider};
use super::view;
use super::{Function, Input, StreamCase};

const CLOCK: i64 = 1_700_000_000;
const STREAM_TIMESTAMP: f64 = 1_700_000_000.0;

const fn now() -> StreamInstant {
    StreamInstant {
        seconds: CLOCK,
        timestamp: STREAM_TIMESTAMP,
    }
}

fn block_on<Fut>(future: Fut) -> Result<Fut::Output, Box<HelperError>>
where
    Fut: Future,
{
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    Ok(runtime.block_on(future))
}

fn one(function: &Function, case: &StreamCase) -> Result<Json, Box<HelperError>> {
    let source = ScriptedSource::new(case)?;
    let sources = ScriptedSources::new(&case.exchange)?;
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
        dynamic_params: case.dynamic_params,
        data: &case.data,
        model: &case.model,
        timeout: case.timeout,
        max_retries: case.max_retries,
        azure_ad_token: token.as_ref(),
        azure_ad_token_provider: provider.as_ref(),
        litellm_params: &litellm_params,
        supplied: Some(&client),
    };
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
        Ok(stream) => stream_outcome(&source, &sources, &recorder, stream),
        Err(error) => view::error_report(&source, &sources, &recorder, &error),
    }
}

fn stream_outcome(
    source: &ScriptedSource,
    sources: &ScriptedSources,
    recorder: &Recorder,
    stream: AzureChatStream,
) -> Result<Json, Box<HelperError>> {
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
        source,
        sources,
        recorder,
        view::Outcome {
            name: "stream",
            wrapper,
            chunks: Json::List(chunks),
            error: Json::Null,
        },
    ))
}

pub(super) fn run(input: &Input) -> HelperResult {
    let values = input
        .cases
        .iter()
        .map(|case| one(&input.function, case))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(view::calls(values))
}
