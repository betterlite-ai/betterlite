use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use betterlite_logging::private_json::files::PrivateFiles;
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

use super::cache::{Clock, SessionCache, load_session};
use super::credentials::{Agent, printable, resolve_credentials};
use super::error::StatuslineError;
use super::fetch::SessionFetch;
use super::render::render;
use super::transcript::{latest_transcript_model, model_label};
use super::wire::{Wire, text};

const CODEX_STOP_EVENT: &str = "Stop";
const FALLBACK_LABEL: &str = "claude";
const CACHE_DIRECTORY: &str = "litellm-statusline";
const CONFIG_DIRECTORY: &str = ".claude";

pub struct Dependencies<'deps> {
    pub environment: &'deps BTreeMap<String, String>,
    pub home_dir: &'deps Path,
    pub temp_dir: &'deps Path,
    pub owner_uid: Option<u32>,
    pub fetch: &'deps dyn SessionFetch,
    pub clock: &'deps dyn Clock,
    pub files: &'deps PrivateFiles,
    pub runtime: &'deps Runtime,
}

#[derive(Serialize)]
struct StopMessage {
    #[serde(rename = "systemMessage")]
    system_message: String,
}

// The hook payload is typed, and a field of the wrong type reads as absent, as LiteLLM's getters do
#[derive(Default, Deserialize)]
struct HookPayload {
    hook_event_name: Option<Wire<String>>,
    transcript_path: Option<Wire<String>>,
    session_id: Option<Wire<String>>,
    model: Option<Wire<Model>>,
}

#[derive(Deserialize)]
struct Model {
    display_name: Option<Wire<String>>,
}
fn display_name(body: &HookPayload) -> String {
    let Some(Wire::Value(model)) = &body.model else {
        return String::new();
    };
    printable(text(model.display_name.as_ref()))
}
fn color_enabled(environment: &BTreeMap<String, String>) -> bool {
    !environment.contains_key("NO_COLOR")
        && !matches!(
            environment.get("TERM").map(String::as_str),
            None | Some("" | "dumb")
        )
}

fn non_empty<'env>(environment: &'env BTreeMap<String, String>, name: &str) -> Option<&'env str> {
    environment
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}
// None: the transcript names no served model, so the caller shows the label Claude Code already had
fn claude_line(
    body: &HookPayload,
    deps: &Dependencies<'_>,
    config_dir: &Path,
    cache_dir: &Path,
) -> Result<Option<String>, StatuslineError> {
    let transcript = text(body.transcript_path.as_ref());
    let Some(served) = latest_transcript_model(Path::new(transcript)) else {
        return Ok(None);
    };
    let label = model_label(&served, config_dir);
    let use_color = color_enabled(deps.environment);
    let session_id = text(body.session_id.as_ref());
    let credentials = resolve_credentials(deps.environment, Agent::ClaudeCode);
    let Some(usable) = credentials.filter(|_| !session_id.is_empty()) else {
        return Ok(Some(render(&label, None, config_dir, use_color)));
    };
    let cache = SessionCache {
        directory: cache_dir,
        owner_uid: deps.owner_uid.unwrap_or(0),
        files: deps.files,
        runtime: deps.runtime,
        clock: deps.clock,
    };
    let session = load_session(&cache, &usable, session_id, deps.fetch)?;
    let routed = session
        .as_ref()
        .map_or(label, |found| model_label(&found.last_model, config_dir));
    Ok(Some(render(
        &routed,
        session.as_ref(),
        config_dir,
        use_color,
    )))
}

// No cache here: the Stop hook runs once per turn, and a first turn's cached absence would hide the record the next turn finds
fn codex_message(
    body: &HookPayload,
    deps: &Dependencies<'_>,
    config_dir: &Path,
) -> Result<Option<String>, StatuslineError> {
    let session_id = text(body.session_id.as_ref());
    let credentials = resolve_credentials(deps.environment, Agent::Codex);
    let Some(usable) = credentials.filter(|_| !session_id.is_empty()) else {
        return Ok(None);
    };
    let Some(session) = deps.fetch.fetch(&usable, session_id)?.session else {
        return Ok(None);
    };
    let line = render(
        &model_label(&session.last_model, config_dir),
        Some(&session),
        config_dir,
        false,
    );
    let message = StopMessage {
        system_message: format!("\n{line}"),
    };
    serde_json::to_string(&message)
        .map(Some)
        .map_err(StatuslineError::Encode)
}

fn cache_directory_name(owner_uid: Option<u32>) -> String {
    owner_uid.map_or_else(
        || CACHE_DIRECTORY.to_owned(),
        |uid| format!("{CACHE_DIRECTORY}-{uid}"),
    )
}

// A failure shows Claude Code the label it already knows and Codex nothing, since a bare string is not hook JSON
pub(crate) fn run_statusline(input: &str, deps: &Dependencies<'_>) -> String {
    let body: HookPayload = serde_json::from_str(input).unwrap_or_default();
    let codex = text(body.hook_event_name.as_ref()) == CODEX_STOP_EVENT;
    let config_dir = non_empty(deps.environment, "CLAUDE_CONFIG_DIR")
        .map_or_else(|| deps.home_dir.join(CONFIG_DIRECTORY), PathBuf::from);
    let temp_root = ["TMPDIR", "TEMP", "TMP"]
        .iter()
        .find_map(|name| non_empty(deps.environment, name))
        .map_or_else(|| deps.temp_dir.to_path_buf(), PathBuf::from);
    let cache_dir = temp_root.join(cache_directory_name(deps.owner_uid));
    let outcome = if codex {
        codex_message(&body, deps, &config_dir)
    } else {
        claude_line(&body, deps, &config_dir, &cache_dir)
    };
    if let Ok(Some(line)) = outcome {
        return line;
    }
    if codex {
        return String::new();
    }
    let shown = display_name(&body);
    if shown.is_empty() {
        FALLBACK_LABEL.to_owned()
    } else {
        shown
    }
}

mod tests;
