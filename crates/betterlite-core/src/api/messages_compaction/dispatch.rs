use std::error::Error;
use std::future::Future;
use std::mem::take;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;

use crate::api::messages_compaction::error::DispatchError;
use crate::api::messages_compaction::normalize::normalize_context_edit_shapes;
use crate::api::messages_context_clear::constants::{CLEAR_TOOL_USES_EDIT_TYPE, COMPACT_EDIT_TYPE};
use crate::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, PolyfillResult, UsageIteration,
};
use crate::api::summary_execution::completion::SummaryCompletion;

// the five keyword arguments a sync editor receives, and nothing else
pub struct ClearCall {
    pub model: String,
    pub messages: Vec<Json>,
    pub tools: Option<Vec<Json>>,
    pub system: Option<Json>,
    pub edit_spec: Json,
}

// the async editor's arguments: the same history plus the three request-wide values only it accepts
pub struct CompactCall<'call> {
    pub model: &'call str,
    pub messages: Vec<Json>,
    pub tools: Option<Vec<Json>>,
    pub system: Option<Json>,
    pub edit_spec: Json,
    pub litellm_metadata: Option<&'call Object>,
    pub llm_router: Option<&'call dyn SummaryCompletion>,
    pub user_api_key_auth: Option<&'call UserAPIKeyAuth>,
}

// an editor's native return: the new PolyfillResult, or the legacy (messages, applied) tuple
pub enum EditorOutcome {
    Polyfilled(PolyfillResult),
    Legacy {
        messages: Vec<Json>,
        applied: Option<AppliedEdit>,
    },
}

// the registry LiteLLM builds from its two editors; anything else is an unknown type
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditType {
    ClearToolUses,
    Compact,
}

impl EditType {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            CLEAR_TOOL_USES_EDIT_TYPE => Some(Self::ClearToolUses),
            COMPACT_EDIT_TYPE => Some(Self::Compact),
            _ => None,
        }
    }
}

// the typed arguments of one registry entry: the sync clear call or the async compact call
pub enum EditCall<'call> {
    Clear(ClearCall),
    Compact(CompactCall<'call>),
}

// the two registered editors behind the dispatcher's single serial loop
pub trait ContextEditor: Send + Sync {
    type Failure: Error + 'static;

    fn edit(
        &self,
        call: EditCall<'_>,
    ) -> impl Future<Output = Result<EditorOutcome, Self::Failure>> + Send;
}

// everything apply_context_management reads from the caller's request
pub struct DispatchRequest<'request> {
    pub model: &'request str,
    pub messages: Vec<Json>,
    pub tools: Option<Vec<Json>>,
    pub system: Option<Json>,
    pub context_management_spec: Option<&'request Json>,
    pub litellm_metadata: Option<&'request Object>,
    pub llm_router: Option<&'request dyn SummaryCompletion>,
    pub user_api_key_auth: Option<&'request UserAPIKeyAuth>,
}

// the parts every edit in one request shares; the history itself moves through State
struct Shared<'shared> {
    model: &'shared str,
    tools: Option<Vec<Json>>,
    litellm_metadata: Option<&'shared Object>,
    llm_router: Option<&'shared dyn SummaryCompletion>,
    user_api_key_auth: Option<&'shared UserAPIKeyAuth>,
}

struct State {
    messages: Vec<Json>,
    system: Option<Json>,
    applied_edits: Vec<AppliedEdit>,
    compaction_block: Option<CompactionBlock>,
    iterations_usage: Option<Vec<UsageIteration>>,
}

impl State {
    // _wrap_editor_return plus the aggregation: a legacy tuple keeps the incoming system, a PolyfillResult keeps its own
    fn carry(&mut self, outcome: EditorOutcome, fallback_system: Option<Json>) {
        match outcome {
            EditorOutcome::Polyfilled(result) => {
                self.messages = result.messages;
                self.system = result.system;
                self.applied_edits.extend(result.applied_edits);
                if result.compaction_block.is_some() {
                    self.compaction_block = result.compaction_block;
                }
                // the most recent non-null usage list wins even when it is empty
                if result.iterations_usage.is_some() {
                    self.iterations_usage = result.iterations_usage;
                }
            }
            EditorOutcome::Legacy { messages, applied } => {
                self.messages = messages;
                self.system = fallback_system;
                if let Some(edit) = applied {
                    self.applied_edits.push(edit);
                }
            }
        }
    }
}

pub async fn apply_context_management<Editor>(
    request: DispatchRequest<'_>,
    editor: &Editor,
) -> Result<PolyfillResult, DispatchError<Editor::Failure>>
where
    Editor: ContextEditor,
{
    let edits = normalize_context_edit_shapes(request.context_management_spec)?;
    let mut state = State {
        messages: request.messages,
        system: request.system,
        applied_edits: Vec::new(),
        compaction_block: None,
        iterations_usage: None,
    };
    let shared = Shared {
        model: request.model,
        tools: request.tools,
        litellm_metadata: request.litellm_metadata,
        llm_router: request.llm_router,
        user_api_key_auth: request.user_api_key_auth,
    };
    for edit_spec in edits {
        run_edit(&shared, editor, &mut state, edit_spec).await?;
    }
    Ok(PolyfillResult {
        messages: state.messages,
        system: state.system,
        applied_edits: state.applied_edits,
        compaction_block: state.compaction_block,
        iterations_usage: state.iterations_usage,
    })
}

// one iteration: the registry lookup, the typed call, and the carry of whatever the editor returned
async fn run_edit<Editor>(
    shared: &Shared<'_>,
    editor: &Editor,
    state: &mut State,
    edit_spec: IndexMap<String, Json>,
) -> Result<(), DispatchError<Editor::Failure>>
where
    Editor: ContextEditor,
{
    let edit_type = match edit_spec.get("type") {
        Some(Json::Text(name)) => EditType::from_name(name),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    };
    let Some(edit_type) = edit_type else {
        tracing::debug!(
            "context_management polyfill: unknown edit type '{}' \u{2014} skipping",
            displayed_type(edit_spec.get("type"))
        );
        return Ok(());
    };
    let fallback_system = state.system.take();
    let messages = take(&mut state.messages);
    let call = edit_call(
        edit_type,
        shared,
        messages,
        fallback_system.clone(),
        edit_spec,
    );
    let outcome = editor.edit(call).await.map_err(DispatchError::Editor)?;
    state.carry(outcome, fallback_system);
    Ok(())
}

// one registry entry's typed arguments: the history and system the previous edit left behind
fn edit_call<'call>(
    edit_type: EditType,
    shared: &Shared<'call>,
    messages: Vec<Json>,
    system: Option<Json>,
    edit_spec: IndexMap<String, Json>,
) -> EditCall<'call> {
    match edit_type {
        EditType::ClearToolUses => EditCall::Clear(ClearCall {
            model: shared.model.to_owned(),
            messages,
            tools: shared.tools.clone(),
            system,
            edit_spec: Json::Object(edit_spec),
        }),
        EditType::Compact => EditCall::Compact(CompactCall {
            model: shared.model,
            messages,
            tools: shared.tools.clone(),
            system,
            edit_spec: Json::Object(edit_spec),
            litellm_metadata: shared.litellm_metadata,
            llm_router: shared.llm_router,
            user_api_key_auth: shared.user_api_key_auth,
        }),
    }
}

// the '%s' of LiteLLM's debug line: scalars read as Python prints them, and a container has no repr to copy
fn displayed_type(value: Option<&Json>) -> String {
    match value {
        None | Some(Json::Null) => String::from("None"),
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Number(number)) => number.to_string(),
        Some(Json::Bool(true)) => String::from("True"),
        Some(Json::Bool(false)) => String::from("False"),
        Some(value @ (Json::List(_) | Json::Object(_))) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}
