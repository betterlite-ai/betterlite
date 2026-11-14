use std::collections::BTreeSet;

use serde::Serialize;

use betterlite_harness::cli_handler::setup::copy_skills;
use betterlite_harness::context::SessionContext;
use betterlite_harness::deepagents::sandbox_backend::{TOOL_FILTER_NAME, ToolFilterMiddleware};
use betterlite_harness::deepagents::transformation::SKILLS_DIR;
use betterlite_harness::deepagents::transformation::gateway::chat_model_kwargs;
use betterlite_harness::deepagents::transformation::tools::{
    EXECUTE_TOOLS, WRITE_TOOLS, blocked_tools, interrupt_config,
};
use betterlite_harness::errors::HarnessError;
use betterlite_harness::options::{DeepAgentsOptions, OptionsKind as _};
use betterlite_harness::sandbox::Sandbox;
use betterlite_harness::transformation::BaseHarnessConfig;
use betterlite_harness::types::JsonObject;

use crate::sandbox_backend::Permissions;

use super::DeepAgentsHandler;
use super::graph::{AgentRuntime, AgentSpec};

const MIDDLEWARE_KEY: &str = "middleware";

pub(super) async fn start<C, R, S>(
    handler: &mut DeepAgentsHandler<C, R>,
    context: &SessionContext<S>,
) -> Result<(), HarnessError>
where
    C: BaseHarnessConfig + Send + Sync,
    R: AgentRuntime,
    S: Sandbox,
{
    handler.config.validate_environment(context)?;
    handler.runtime.load_deps()?;
    let blocked = blocked_tools(
        context.settings.permissions,
        &context.settings.disable_tools,
    );
    let spec = agent_spec(handler, context, &blocked).await?;
    let graph = handler.runtime.build_agent(spec)?;
    handler.skip_tools = skip_tools(context);
    if handler.thread_id.is_none() {
        handler.thread_id = Some(context.session_id.clone());
    }
    handler.graph = Some(graph);
    Ok(())
}

async fn agent_spec<C, R, S>(
    handler: &DeepAgentsHandler<C, R>,
    context: &SessionContext<S>,
    blocked: &BTreeSet<String>,
) -> Result<AgentSpec, HarnessError>
where
    C: BaseHarnessConfig + Send + Sync,
    R: AgentRuntime,
    S: Sandbox,
{
    Ok(AgentSpec {
        chat_model: chat_model_kwargs(context)?,
        system_prompt: context.settings.instructions.clone(),
        middleware: (!blocked.is_empty()).then(|| ToolFilterMiddleware {
            blocked: blocked.clone(),
        }),
        subagents: subagents(handler, context, blocked)?,
        skills: install_skills(context).await?,
        permissions: Permissions {
            writable: WRITE_TOOLS.iter().all(|tool| !blocked.contains(*tool)),
            allow_execute: EXECUTE_TOOLS.iter().all(|tool| !blocked.contains(*tool)),
        },
        interrupt_on: interrupt_config(context.settings.permissions, blocked),
        output_schema: context.settings.output_schema.clone(),
    })
}

fn subagents<C, R, S>(
    handler: &DeepAgentsHandler<C, R>,
    context: &SessionContext<S>,
    blocked: &BTreeSet<String>,
) -> Result<Vec<JsonObject>, HarnessError>
where
    C: BaseHarnessConfig,
    R: AgentRuntime,
{
    let user_subagents = context
        .settings
        .options
        .as_ref()
        .and_then(DeepAgentsOptions::pick)
        .map_or_else(Vec::new, |options| options.subagents.clone());
    let default_subagent = handler.runtime.general_subagent();
    let has_general = default_subagent.get("name").is_some_and(|name| {
        user_subagents
            .iter()
            .any(|subagent| subagent.get("name") == Some(name))
    });
    let mut combined = Vec::new();
    if !blocked.is_empty() && !has_general {
        let mut general = default_subagent;
        let filter = serde_json::to_value([ToolFilterEntry {
            name: TOOL_FILTER_NAME,
            blocked,
        }])
        .map_err(|source| HarnessError::StateEncode { source })?;
        general.insert(MIDDLEWARE_KEY.to_owned(), filter);
        combined.push(general);
    }
    combined.extend(user_subagents);
    Ok(combined)
}

async fn install_skills<S>(context: &SessionContext<S>) -> Result<Option<Vec<String>>, HarnessError>
where
    S: Sandbox,
{
    if context.settings.skills.is_empty() {
        return Ok(None);
    }
    let skills_root = format!("{}/{SKILLS_DIR}", context.sandbox.workdir());
    copy_skills(&context.sandbox, &context.settings.skills, &skills_root).await?;
    Ok(Some(vec![format!("/{SKILLS_DIR}/")]))
}

fn skip_tools<S>(context: &SessionContext<S>) -> BTreeSet<String> {
    context
        .settings
        .output_schema
        .as_ref()
        .and_then(|schema| schema.get("title"))
        .and_then(|title| title.as_str())
        .map_or_else(BTreeSet::new, |title| BTreeSet::from([title.to_owned()]))
}

// the runtime turns this entry back into deepagents' ToolFilterMiddleware for the default subagent
#[derive(Serialize)]
struct ToolFilterEntry<'blocked> {
    name: &'static str,
    blocked: &'blocked BTreeSet<String>,
}
