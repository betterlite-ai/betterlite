use std::collections::BTreeMap;

use betterlite_core::api::response_id_traversal::response_id::transcode;
use betterlite_core::api::responses_create::call::ResponsesCreateCall;
use betterlite_core::api::responses_create::deps::ResponsesCreateDeps;
use betterlite_core::api::responses_create_call::aresponses;
use betterlite_core::api::responses_create_call::deps::{AsyncCreateDeps, CreateExecution};
use betterlite_core::api::responses_create_http::service::{CreateHooks, LogContext};
use betterlite_core::api::responses_hook_policy::callbacks::CallbackLists;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{
    Configs, DispatchFailure, DispatchResolver, FixtureError, LoggingProbe, NoBridge,
    NoInterception, Nothing, Outbound,
};

struct DispatchPorts {
    resolver: DispatchResolver,
    configs: Configs,
    nothing: Nothing,
    interception: NoInterception,
    bridge: NoBridge,
    variables: BTreeMap<String, String>,
}

// The HTTP call LiteLLM makes for the turn, through the production aresponses path, with the answer discarded
pub(crate) async fn dispatch(
    outbound: &Outbound,
    base_url: &str,
    model: &str,
    call_kwargs: &IndexMap<String, Json>,
) -> Result<(), DispatchFailure> {
    let call = call_for_dispatch(base_url, model, call_kwargs)?;
    let mut probe = LoggingProbe::new()?;
    let environment = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &probe.redactor,
        log: &probe.sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: probe.now,
    };
    let mut log = LogContext {
        state: &mut probe.state,
        env: &environment,
    };
    let ports = DispatchPorts {
        resolver: DispatchResolver,
        configs: Configs,
        nothing: Nothing,
        interception: NoInterception,
        bridge: NoBridge,
        variables: BTreeMap::new(),
    };
    let deps = dependencies(outbound, &ports);
    let execution = CreateExecution { deps: &deps };
    let create = AsyncCreateDeps {
        resolver: &ports.resolver,
        lookup: &ports.configs,
        prompts: None,
    };
    let _created = aresponses(&create, &execution, &call, (), &mut log).await?;
    Ok(())
}

fn call_for_dispatch(
    base_url: &str,
    model: &str,
    call_kwargs: &IndexMap<String, Json>,
) -> Result<ResponsesCreateCall, FixtureError> {
    transcode(&Json::Object(dispatched(base_url, model, call_kwargs))).map_err(FixtureError::Call)
}

fn dependencies<'deps>(
    outbound: &'deps Outbound,
    ports: &'deps DispatchPorts,
) -> ResponsesCreateDeps<'deps, Outbound, Configs, Nothing, Nothing, ()> {
    ResponsesCreateDeps {
        resolver: &ports.resolver,
        lookup: &ports.configs,
        configs: &ports.configs,
        source: outbound,
        supplied: None,
        environment: &ports.variables,
        prompts: None,
        file_mapping: None,
        hooks: CreateHooks {
            callbacks: CallbackLists {
                global: &[],
                dynamic: &[],
            },
            resolver: &ports.nothing,
            dispatcher: &ports.nothing,
            executor: &ports.nothing,
            logging: None,
        },
        interception: &ports.interception,
        bridge: &ports.bridge,
        drop_params: false,
        reasoning_auto_summary: false,
    }
}

fn dispatched(
    base_url: &str,
    model: &str,
    call_kwargs: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut fields = call_kwargs.clone();
    fields.insert(String::from("model"), Json::Text(model.to_owned()));
    fields.insert(
        String::from("api_base"),
        Json::Text(format!("{base_url}/v1")),
    );
    fields.insert(
        String::from("api_key"),
        Json::Text(String::from("sk-fixture")),
    );
    // the mock answers only streamed events, so the dispatch asks for a stream whatever the turn does
    fields.insert(String::from("stream"), Json::Bool(true));
    fields
}
