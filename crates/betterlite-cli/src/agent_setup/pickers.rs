use super::error::AgentSetupError;
use super::prompt::Prompter;

pub const TARGET_EDIT_MESSAGE: &str =
    "Which agents should be edited? Unselected agents keep their current setup";
pub const TARGET_ROUTE_MESSAGE: &str = "Which agents should route through LiteLLM?";
pub const EMPTY_TARGETS_MESSAGE: &str = "Pick at least one.";
pub const CLAUDE_MODEL_MESSAGE: &str =
    "Model Claude Code starts on (type to filter; /model switches any time):";
pub const CODEX_MODEL_MESSAGE: &str = "Model Codex starts on (type to filter):";
pub const KEEP_CLAUDE_DEFAULT: &str = "Keep Claude Code's own default";

// LiteLLM preselects the saved model only when the listing serves it, and the keep row otherwise
pub fn pick_claude_model(
    prompter: &mut dyn Prompter,
    names: &[String],
    default: Option<&str>,
) -> Result<Option<String>, AgentSetupError> {
    let mut choices = Vec::with_capacity(names.len().saturating_add(1));
    choices.push(KEEP_CLAUDE_DEFAULT.to_owned());
    choices.extend(names.iter().cloned());
    let selected = default
        .and_then(|value| names.iter().position(|name| name == value))
        .map_or(0, |index| index.saturating_add(1));
    let picked = prompter.pick_model(CLAUDE_MODEL_MESSAGE, &choices, selected)?;
    if picked == KEEP_CLAUDE_DEFAULT {
        Ok(None)
    } else {
        Ok(Some(picked))
    }
}

pub fn pick_codex_model(
    prompter: &mut dyn Prompter,
    ids: &[String],
    default: Option<&str>,
) -> Result<String, AgentSetupError> {
    let selected = default
        .and_then(|value| ids.iter().position(|id| id == value))
        .unwrap_or_default();
    prompter.pick_model(CODEX_MODEL_MESSAGE, ids, selected)
}
