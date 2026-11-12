use betterlite_anthropic::native_compaction::anthropic::AnthropicCompaction;
use betterlite_anthropic::native_compaction::blocks::native_blocks;
use betterlite_anthropic::native_compaction::dispatch::dispatch;
use betterlite_anthropic::native_compaction::protocol::{
    CompactionParams, CompactionProtocol, Mapping, NativeCompactionProvider as _,
    get_native_compaction_provider,
};
use betterlite_models::catalog::BundledCatalog;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{HelperError, HelperResult};
use fakes::{CatalogLookup, RecordingRouter};

mod fakes;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Supports {
        model: String,
        custom_llm_provider: Option<String>,
    },
    Defaults {
        payload: Mapping,
    },
    RequestKwargs,
    Blocks {
        protocol: CompactionProtocol,
        response: Mapping,
    },
    Summary {
        protocol: CompactionProtocol,
        response: Mapping,
    },
    Dispatch {
        protocol: CompactionProtocol,
        payload: Mapping,
        reply_id: String,
    },
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "NativeCompaction".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn supports(
    runtime: &Runtime,
    lookup: &CatalogLookup,
    model: &str,
    custom_llm_provider: Option<&str>,
) -> serde_json::Value {
    let params = CompactionParams {
        model,
        custom_llm_provider,
    };
    json!(
        runtime
            .block_on(get_native_compaction_provider(lookup, &params))
            .is_some()
    )
}

fn dispatch_call(
    runtime: &Runtime,
    protocol: CompactionProtocol,
    payload: &Mapping,
    reply_id: &str,
) -> HelperResult {
    let router = RecordingRouter::new(reply_id.to_owned());
    let reply = runtime
        .block_on(dispatch(&router, protocol, payload))
        .map_err(|error| failure(error.to_string()))?;
    Ok(json!({"calls": router.recorded(), "result_id": reply.get("id")}))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        let lookup = CatalogLookup {
            catalog: BundledCatalog::default().get(),
        };
        let provider = AnthropicCompaction { lookup: &lookup };
        match input {
            Input::Supports {
                model,
                custom_llm_provider,
            } => Ok(supports(
                &runtime,
                &lookup,
                model,
                custom_llm_provider.as_deref(),
            )),
            Input::Defaults { payload } => Ok(json!(provider.compatible_defaults(payload))),
            Input::RequestKwargs => serde_json::to_value(provider.request_kwargs())
                .map_err(|error| failure(error.to_string())),
            Input::Blocks { protocol, response } => native_blocks(*protocol, response)
                .map(|blocks| json!(blocks))
                .map_err(|error| failure(error.to_string())),
            Input::Summary { protocol, response } => provider
                .extract_summary(*protocol, response)
                .map(|summary| json!(summary))
                .map_err(|error| failure(error.to_string())),
            Input::Dispatch {
                protocol,
                payload,
                reply_id,
            } => dispatch_call(&runtime, *protocol, payload, reply_id),
        }
    }
}
