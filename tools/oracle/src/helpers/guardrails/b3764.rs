use std::time::Duration;

use betterlite_context::agentic_callback::{AgenticCall, AgenticLogging, AgenticLoopCall};
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use betterlite_guardrails::compresr_recovery::plan::{PlanRequest, build_plan, should_run};
use betterlite_guardrails::compresr_recovery::store::{Originals, RecoveryStore, StoreKey};
use serde::Deserialize;
use serde_json::json;
use serde_json::value::{RawValue, to_raw_value};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

// the marker LiteLLM would draw a per-process token for, standing in for the guardrail's own
const OWN_MARKER: &str = "$OWN";

// What the proxy puts into the request data once it has authenticated the virtual key
fn carried_kwargs(kwargs: &JsonMap, scope: &str) -> Result<JsonMap, Box<HelperError>> {
    let mut carried = kwargs.clone();
    let carrier = json!({"metadata": {"user_api_key_auth": {"api_key": scope}}});
    carried.insert(
        "litellm_params".to_owned(),
        to_raw_value(&carrier).or_config_error()?,
    );
    Ok(carried)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Seed {
    scope: String,
    call_id: String,
    originals: Originals,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    scope: String,
    call_id: String,
    model: String,
    messages: Vec<FollowupMessage>,
    response: Box<RawValue>,
    request_tools: Option<Vec<FollowupMessage>>,
    optional_params: JsonMap,
    kwargs: JsonMap,
    #[serde(default)]
    agentic_loop_params: Option<JsonMap>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    seed: Seed,
    call: Call,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let store = RecoveryStore::default();
        let seed = &input.seed;
        if let Some(seed_key) = StoreKey::resolve(Some(&seed.call_id), &seed.scope) {
            store.store(&seed_key, seed.originals.clone(), Duration::ZERO, 0);
        }
        let call = &input.call;
        let kwargs = carried_kwargs(&call.kwargs, &call.scope)?;
        let gate_call = AgenticCall {
            response: &call.response,
            model: &call.model,
            messages: &call.messages,
            tools: call.request_tools.as_deref(),
            stream: false,
            custom_llm_provider: "openai",
            kwargs: &kwargs,
        };
        let gate = should_run(&gate_call).or_config_error()?;
        let logging = AgenticLogging {
            call_id: Some(call.call_id.clone()),
            agentic_loop_params: call.agentic_loop_params.clone(),
        };
        let request = PlanRequest {
            call: AgenticLoopCall {
                tools: &gate.tools,
                model: &call.model,
                messages: &call.messages,
                response: &call.response,
                optional_params: &call.optional_params,
                logging: Some(&logging),
                stream: false,
                kwargs: &kwargs,
            },
            own_marker: Some(OWN_MARKER),
            now: Duration::ZERO,
        };
        let plan = build_plan(&store, &request).or_config_error()?;
        Ok(json!({"gate": {"run": gate.run, "tools": gate.tools}, "plan": plan}))
    }
}
