pub mod args;
pub mod call;
pub mod error;
pub mod ports;
mod steps;
mod tests;
use self::args::{CompletionArgs, CompletionSettings, JsonObject};
use self::call::{FallbackCall, PreparedCall, PreparedCompletion};
use self::error::{CompletionRequestError, RequestResult};
use self::ports::CompletionPorts;
use crate::completion_options::control::resolve_control_options;
use betterlite_core::message_utilities::validators::StopSequences;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
use std::time::Duration;
use tokio::time::sleep;
pub(crate) struct Preparation<'request> {
    pub args: &'request CompletionArgs,
    pub settings: &'request CompletionSettings,
    pub ports: &'request CompletionPorts<'request>,
    pub call: PreparedCall,
    pub kwargs: JsonObject,
    pub non_default: JsonObject,
    pub snapshot: CompletionArgs,
    pub tool_choice: Option<Json>,
    pub stop: Option<StopSequences>,
    pub thinking: Option<Json>,
    pub skip_bridge: bool,
    pub control: ControlOptions,
    pub acompletion: bool,
}
pub async fn prepare_completion<'call, 'ports>(
    args: &'call CompletionArgs,
    settings: &'call CompletionSettings,
    ports: &'call CompletionPorts<'ports>,
) -> RequestResult<PreparedCompletion>
where
    'ports: 'call,
{
    let mut state = Preparation {
        args,
        settings,
        ports,
        call: PreparedCall::default(),
        kwargs: args.kwargs.clone(),
        non_default: JsonObject::new(),
        snapshot: CompletionArgs::default(),
        tool_choice: None,
        stop: None,
        thinking: None,
        skip_bridge: false,
        control: ControlOptions::default(),
        acompletion: false,
    };
    run(&mut state).await
}
async fn run(state: &mut Preparation<'_>) -> RequestResult<PreparedCompletion> {
    steps::messages::validate(state)?;
    if let Some(outcome) = steps::params::flags(state)? {
        return Ok(outcome);
    }
    steps::headers::merge(state)?;
    steps::messages::alternation(state)?;
    steps::params::non_default(state)?;
    steps::params::cache_control(state)?;
    steps::params::prompt_hook(state, steps::params::HookForm::Blocking).await?;
    steps::messages::system_prompt(state)?;
    steps::params::router_params(state)?;
    if let Some(outcome) = steps::provider::early_exit(state)? {
        return Ok(outcome);
    }
    let provider = steps::provider::resolve(state)?;
    steps::bridge::early(state, &provider)?;
    steps::messages::drop_tool_keys(state, &provider);
    steps::headers::provider_specific(state, &provider)?;
    steps::provider::hidden_params(state, &provider);
    let timeout = steps::provider::timeout(state, &provider)?;
    state.call.timeout = Some(timeout);
    steps::provider::pricing(state, &provider).await?;
    steps::provider::custom_prompt(state);
    steps::messages::file_ids(state)?;
    steps::messages::developer_role(state, &provider);
    steps::messages::system_messages(state)?;
    steps::assemble::assemble(state, &provider)?;
    steps::finish::litellm_params(state)?;
    steps::headers::affinity(state)?;
    steps::finish::environment(state);
    steps::finish::mock_or_bridge(state, &provider)
}
pub async fn prepare_acompletion<'call, 'ports>(
    args: &'call CompletionArgs,
    settings: &'call CompletionSettings,
    ports: &'call CompletionPorts<'ports>,
) -> RequestResult<PreparedCompletion>
where
    'ports: 'call,
{
    let Some(model) = args.model.clone() else {
        return Err(CompletionRequestError::ModelNotPassed);
    };
    if steps::messages::optional::<bool>(&args.kwargs, "mock_timeout")? == Some(true)
        && let Some(timeout) = &args.timeout
    {
        sleep(timeout_duration(timeout)?).await;
        return Err(CompletionRequestError::MockTimeout);
    }
    let control = resolve_control_options(&args.kwargs, &model, settings.drop_params)?;
    let mut head = Preparation {
        args,
        settings,
        ports,
        call: PreparedCall::default(),
        kwargs: args.kwargs.clone(),
        non_default: args.kwargs.clone(),
        snapshot: CompletionArgs::default(),
        tool_choice: None,
        stop: None,
        thinking: None,
        skip_bridge: false,
        control,
        acompletion: true,
    };
    steps::messages::validate(&mut head)?;
    steps::params::cache_control(&mut head)?;
    steps::params::prompt_hook(&mut head, steps::params::HookForm::Async).await?;
    let working = working_args(&head, args)?;
    if let Some(fallbacks) = steps::provider::fallbacks_for(&working, settings)? {
        return Ok(PreparedCompletion::Fallbacks(FallbackCall {
            fallbacks,
            args: working,
        }));
    }
    prepare_completion(&working, settings, ports).await
}
fn timeout_duration(timeout: &Json) -> RequestResult<Duration> {
    let number = match timeout {
        Json::Number(number) => number.as_f64(),
        Json::Text(text) => text.parse::<f64>().ok(),
        Json::Object(fields) => match fields.get("connect") {
            None | Some(Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_)) => {
                Some(0.0_f64)
            }
            Some(Json::Number(number)) => number.as_f64(),
            Some(Json::Text(_)) => None,
        },
        Json::Null | Json::Bool(_) | Json::List(_) => None,
    };
    let count = number.ok_or(CompletionRequestError::FieldType {
        field: "timeout",
        expected: "a number of seconds",
    })?;
    if !count.is_finite() {
        return Err(CompletionRequestError::FieldType {
            field: "timeout",
            expected: "a finite number of seconds",
        });
    }
    if count < 0.0_f64 {
        return Err(CompletionRequestError::FieldType {
            field: "timeout",
            expected: "a non-negative number of seconds",
        });
    }
    Duration::try_from_secs_f64(count).map_err(|_source| CompletionRequestError::FieldType {
        field: "timeout",
        expected: "a number of seconds within range",
    })
}
fn working_args(head: &Preparation<'_>, args: &CompletionArgs) -> RequestResult<CompletionArgs> {
    let mut working = args.clone();
    working.model = Some(head.call.model.clone());
    working.messages = steps::convert::convert(&head.call.messages)?;
    working.tools.clone_from(&head.call.tools);
    working.kwargs.clone_from(&head.non_default);
    working
        .kwargs
        .insert(String::from("acompletion"), Json::Bool(true));
    if steps::messages::optional::<String>(&working.kwargs, "custom_llm_provider")?.is_none() {
        let provider = steps::params::resolve_head_provider(head)?;
        working
            .kwargs
            .insert(String::from("custom_llm_provider"), Json::Text(provider));
    }
    Ok(working)
}
