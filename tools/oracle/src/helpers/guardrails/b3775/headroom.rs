use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_context::agentic_callback::{
    AgenticCall, AgenticCallback as _, AgenticLogging, AgenticLoopCall,
};
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, FollowupMessage, JsonMap};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract as _;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::headroom::headroom_compression::{HeadroomGuardrail, HeadroomRuntime};
use betterlite_guardrails::headroom_settings::{HeadroomInput, HeadroomSettings};
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use uuid::Uuid;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_oracle_runner::case::UpstreamAnswer;
use betterlite_oracle_runner::mock::MockProvider;
use betterlite_oracle_runner::routing_mock_scripts::{
    MockScript, Outcome, ProviderCall, ScriptSpec, ScriptedReply, Wait,
};

use super::scripted::Reply;
use tokio::runtime::{Builder, Runtime};

const UNREACHABLE: &str = "http://127.0.0.1:1";
const CALL_ID_KEY: &str = "litellm_call_id";
const MODEL: &str = "gpt-4o";
const TIMEOUT_MILLIS: u64 = 300;
const DEADLINE_MILLIS: u64 = 30_000;
// one compressible row: LiteLLM protects the system, last user and assistant rows
const SEED_INPUTS: &str = r#"{"structured_messages":[{"role":"system","content":"sys"},{"role":"user","content":"long text"},{"role":"assistant","content":"answer"},{"role":"user","content":"now"}]}"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShouldRun {
    response: Box<RawValue>,
    tools: Option<Vec<FollowupMessage>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Seed {
    call_id: String,
    hashes: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoggingInput {
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    agentic_loop_params: Option<JsonMap>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    timeout: Option<f64>,
    seed: Seed,
    messages: Vec<FollowupMessage>,
    tools: JsonMap,
    model: String,
    response: Box<RawValue>,
    #[serde(default)]
    optional_params: JsonMap,
    #[serde(default)]
    kwargs: JsonMap,
    #[serde(default)]
    logging: Option<LoggingInput>,
    replies: Vec<Reply>,
}

pub(crate) fn should_run(input: &ShouldRun) -> HelperResult {
    let guardrail = build(UNREACHABLE, None, None)?;
    let messages: Vec<FollowupMessage> = Vec::new();
    let kwargs = JsonMap::new();
    let call = AgenticCall {
        response: &input.response,
        model: MODEL,
        messages: &messages,
        tools: input.tools.as_deref(),
        stream: false,
        custom_llm_provider: "openai",
        kwargs: &kwargs,
    };
    let gate = runtime()?
        .block_on(guardrail.should_run_agentic_loop(&call))
        .or_config_error()?;
    Ok(json!({"run": gate.run, "tools": gate.tools}))
}

pub(crate) fn plan(input: &Plan) -> HelperResult {
    let mock = MockProvider::start(script(input)?).or_config_error()?;
    let base = mock.base_url.clone();
    let guardrail = build(&base, input.api_key.clone(), input.timeout)?;
    seed_hashes(&guardrail, &input.seed)?;
    let logging = input.logging.as_ref().map(|logging| AgenticLogging {
        call_id: logging.call_id.clone(),
        agentic_loop_params: logging.agentic_loop_params.clone(),
    });
    let call = AgenticLoopCall {
        tools: &input.tools,
        model: &input.model,
        messages: &input.messages,
        response: &input.response,
        optional_params: &input.optional_params,
        logging: logging.as_ref(),
        stream: false,
        kwargs: &input.kwargs,
    };
    let built = runtime()?.block_on(guardrail.build_agentic_loop_plan(&call));
    let calls = mock.finish().or_config_error()?;
    let plan: AgenticLoopPlan = built.or_config_error()?;
    Ok(json!({"plan": plan, "calls": described(&calls)}))
}

fn build(
    base: &str,
    api_key: Option<String>,
    timeout: Option<f64>,
) -> Result<HeadroomGuardrail, Box<HelperError>> {
    let input = HeadroomInput {
        api_base: Some(base.to_owned()),
        api_key: api_key.map(SecretString::from),
        model: None,
        unreachable_fallback: None,
        timeout,
        ccr_retrieval: Some(true),
    };
    let settings =
        HeadroomSettings::new(&input, &BTreeMap::<String, String>::new()).or_config_error()?;
    HeadroomGuardrail::new(
        Some("headroom".to_owned()),
        settings,
        Box::new(|connect: Duration| base_builder().connect_timeout(connect).build()),
        HeadroomRuntime {
            clock: Box::new(|| Duration::ZERO),
            wall: SystemTime::now,
            new_id: Uuid::new_v4,
            event_hook: None,
        },
    )
    .or_config_error()
}

fn seed_hashes(guardrail: &HeadroomGuardrail, seed: &Seed) -> Result<(), Box<HelperError>> {
    let inputs: GuardrailApiInputs = serde_json::from_str(SEED_INPUTS).or_value_error()?;
    let mut request = BTreeMap::from([(
        CALL_ID_KEY.to_owned(),
        GuardrailValue::Text(seed.call_id.clone()),
    )]);
    runtime()?
        .block_on(guardrail.apply_guardrail(
            inputs,
            &mut request,
            GuardrailInputType::Request,
            None,
        ))
        .or_config_error()?;
    Ok(())
}

fn script(input: &Plan) -> Result<MockScript, Box<HelperError>> {
    let mut replies = vec![ScriptedReply {
        wait: Wait::None,
        then: Outcome::Answer(compress_answer(&input.seed.hashes)),
    }];
    replies.extend(input.replies.iter().map(scripted));
    MockScript::try_from(ScriptSpec {
        deadline_ms: DEADLINE_MILLIS,
        replies,
    })
    .or_config_error()
}

fn compress_answer(hashes: &[String]) -> UpstreamAnswer {
    UpstreamAnswer {
        status: 200,
        headers: BTreeMap::new(),
        body: Some(json!({
            "messages": [{"role": "user", "content": "short"}],
            "ccr_hashes": hashes,
        })),
        events: None,
    }
}

fn scripted(reply: &Reply) -> ScriptedReply {
    match reply {
        Reply::Answer { status, body } => ScriptedReply {
            wait: Wait::None,
            then: Outcome::Answer(answer(*status, body.clone())),
        },
        Reply::Timeout => ScriptedReply {
            wait: Wait::Delay {
                millis: TIMEOUT_MILLIS,
            },
            then: Outcome::Answer(answer(200, json!({"original_content": "late"}))),
        },
        Reply::Close => ScriptedReply {
            wait: Wait::None,
            then: Outcome::Close,
        },
    }
}

const fn answer(status: u16, body: Value) -> UpstreamAnswer {
    UpstreamAnswer {
        status,
        headers: BTreeMap::new(),
        body: Some(body),
        events: None,
    }
}

fn described(calls: &[ProviderCall]) -> Vec<Value> {
    calls
        .iter()
        .map(|call| {
            json!({
                "method": call.request.method,
                "path": call.request.path,
            })
        })
        .collect()
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .or_config_error()
}
