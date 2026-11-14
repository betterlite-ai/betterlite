use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_stream::try_stream;
use futures_core::Stream;
use futures_util::StreamExt as _;
use tokio::sync::SetOnce;

use betterlite_harness::context::SessionContext;
use betterlite_harness::deepagents::sandbox_backend::usage::UsageCallback;
use betterlite_harness::deepagents::transformation::MODEL_NODE;
use betterlite_harness::deepagents::transformation::message::{
    Content, Message, MessageKind, MessageUsage, final_ai_text, stream_events,
};
use betterlite_harness::deepagents::transformation::tools::normalized_tool_name;
use betterlite_harness::deepagents::transformation::update::{
    HitlDecision, Interrupt, TurnState, decision, interrupts_in, structured_json, update_events,
};
use betterlite_harness::errors::HarnessError;
use betterlite_harness::events::{Approval, Decision, Event};
use betterlite_harness::sandbox::Sandbox;
use betterlite_harness::transformation::BaseHarnessConfig;

use super::DeepAgentsHandler;
use super::graph::{AgentRuntime, GraphInput, GraphState, StreamPart};

// async-stream's ? desugaring always calls Into::into to build the stream's error item
#[expect(
    clippy::disallowed_methods,
    reason = "try_stream!'s ? desugaring always calls Into::into to build the yielded Err; ordinary ? elsewhere still uses From::from and stays checked"
)]
pub(super) fn turn<C, R, S>(
    handler: &DeepAgentsHandler<C, R>,
    context: &mut SessionContext<S>,
    prompt: &str,
) -> impl Stream<Item = Result<Event, HarnessError>> + Send
where
    C: BaseHarnessConfig + Send + Sync,
    R: AgentRuntime,
    S: Sandbox,
{
    try_stream! {
        let graph = handler.graph.as_ref().ok_or(HarnessError::SessionClosed)?;
        let usage_callback = handler.runtime.usage_callback(context.settings.model.clone());
        let run_config = handler.run_config(context, usage_callback.clone());
        let mut input = GraphInput::Messages(user_message(prompt));
        loop {
            let mut state = TurnState::default();
            let mut parts = graph.stream(input, run_config.clone());
            while let Some(part) = parts.next().await {
                for event in part_events(part?, &mut state, &handler.skip_tools) {
                    yield event;
                }
            }
            if state.interrupts.is_empty() {
                break;
            }
            let mut resume = BTreeMap::new();
            {
                let mut approvals = Box::pin(approval_events(state.interrupts, &mut resume));
                while let Some(event) = approvals.next().await {
                    yield event?;
                }
            }
            input = GraphInput::Resume(resume);
        }
        finish_turn(context, &graph.state(run_config), usage_callback.as_ref())?;
    }
}

fn user_message(prompt: &str) -> Message {
    Message {
        kind: MessageKind::Human,
        content: Content::Text(prompt.to_owned()),
        reasoning_content: String::new(),
        tool_calls: Vec::new(),
        tool_call_id: String::new(),
        name: None,
        is_error: false,
        usage: MessageUsage::default(),
        reported_cost: None,
    }
}

fn part_events(
    part: StreamPart,
    state: &mut TurnState,
    skip_tools: &BTreeSet<String>,
) -> Vec<Event> {
    match part {
        StreamPart::Message { message, node } => {
            if node == MODEL_NODE {
                return stream_events(&message);
            }
            Vec::new()
        }
        StreamPart::Update(update) => {
            state.interrupts.extend(interrupts_in(&update));
            update_events(&update, skip_tools)
        }
    }
}

fn approval_events(
    interrupts: Vec<Interrupt>,
    resume: &mut BTreeMap<String, Vec<HitlDecision>>,
) -> impl Stream<Item = Result<Event, HarnessError>> + Send {
    try_stream! {
        for interrupt in interrupts {
            let mut decisions = Vec::new();
            for request in &interrupt.value.action_requests {
                let answer = Arc::new(SetOnce::new());
                yield Event::Approval(Approval {
                    tool: normalized_tool_name(&request.name).to_owned(),
                    input: request.args.clone(),
                    decision: Arc::clone(&answer),
                });
                let chosen = answer.wait().await;
                let (allowed, reason) = match chosen {
                    Decision::Allow => (true, ""),
                    Decision::Deny { reason } => (false, reason.as_str()),
                };
                decisions.push(decision(allowed, reason));
            }
            resume.insert(interrupt.id, decisions);
        }
    }
}

fn finish_turn<S>(
    context: &mut SessionContext<S>,
    state: &GraphState,
    usage_callback: Option<&UsageCallback>,
) -> Result<(), HarnessError> {
    context.outcome.final_text = final_ai_text(&state.messages);
    if context.settings.output_schema.is_some() {
        context.outcome.output_json = structured_json(state.structured_response.as_ref())?;
    }
    let Some(totals) = usage_callback.map(UsageCallback::totals) else {
        return Ok(());
    };
    // record_llm_usage never fails a turn, so a count at its type's limit stays there instead of wrapping
    context.outcome.usage.input_tokens = context
        .outcome
        .usage
        .input_tokens
        .saturating_add(totals.usage.input_tokens);
    context.outcome.usage.output_tokens = context
        .outcome
        .usage
        .output_tokens
        .saturating_add(totals.usage.output_tokens);
    context.outcome.usage.calls = context
        .outcome
        .usage
        .calls
        .saturating_add(totals.usage.calls);
    context.outcome.cost = context.outcome.cost.saturating_add(totals.cost);
    Ok(())
}
