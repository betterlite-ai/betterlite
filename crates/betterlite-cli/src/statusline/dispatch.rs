use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use betterlite_logging::private_json::files::PrivateFiles;
use serde::Serialize;
use serde_json::value::RawValue;
use tokio::runtime::Runtime;

use super::cache::{Clock, SessionCache, load_session};
use super::credentials::{Agent, printable, resolve_credentials};
use super::error::StatuslineError;
use super::fetch::SessionFetch;
use super::render::render;
use super::transcript::{latest_transcript_model, model_label};

const CODEX_STOP_EVENT: &str = "Stop";
const FALLBACK_LABEL: &str = "claude";
const CACHE_DIRECTORY_PREFIX: &str = "litellm-statusline-";
const CONFIG_DIRECTORY: &str = ".claude";

pub struct Dependencies<'d> {
    pub environment: &'d BTreeMap<String, String>,
    pub home_dir: &'d Path,
    pub temp_dir: &'d Path,
    pub owner_uid: u32,
    pub fetch: &'d dyn SessionFetch,
    pub clock: &'d dyn Clock,
    pub files: &'d PrivateFiles,
    pub runtime: &'d Runtime,
}

#[derive(Serialize)]
struct StopMessage {
    #[serde(rename = "systemMessage")]
    system_message: String,
}

// A field of the wrong type reads as absent, the way the hook payload is only trusted for what it declares
type Fields = BTreeMap<String, Box<RawValue>>;

fn text_field(body: &Fields, name: &str) -> String {
    body.get(name)
        .and_then(|raw| serde_json::from_str(raw.get()).ok())
        .unwrap_or_default()
}

fn display_name(body: &Fields) -> String {
    let model: Fields = body
        .get("model")
        .and_then(|raw| serde_json::from_str(raw.get()).ok())
        .unwrap_or_default();
    printable(&text_field(&model, "display_name"))
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
    body: &Fields,
    deps: &Dependencies<'_>,
    config_dir: &Path,
    cache_dir: &Path,
) -> Result<Option<String>, StatuslineError> {
    let transcript = text_field(body, "transcript_path");
    let Some(served) = latest_transcript_model(Path::new(&transcript)) else {
        return Ok(None);
    };
    let label = model_label(&served, config_dir);
    let use_color = color_enabled(deps.environment);
    let session_id = text_field(body, "session_id");
    let credentials = resolve_credentials(deps.environment, Agent::ClaudeCode);
    let Some(usable) = credentials.filter(|_| !session_id.is_empty()) else {
        return Ok(Some(render(&label, None, config_dir, use_color)));
    };
    let cache = SessionCache {
        directory: cache_dir,
        owner_uid: deps.owner_uid,
        files: deps.files,
        runtime: deps.runtime,
        clock: deps.clock,
    };
    let session = load_session(&cache, &usable, &session_id, deps.fetch)?;
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
    body: &Fields,
    deps: &Dependencies<'_>,
    config_dir: &Path,
) -> Result<Option<String>, StatuslineError> {
    let session_id = text_field(body, "session_id");
    let credentials = resolve_credentials(deps.environment, Agent::Codex);
    let Some(usable) = credentials.filter(|_| !session_id.is_empty()) else {
        return Ok(None);
    };
    let Some(session) = deps.fetch.fetch(&usable, &session_id)?.session else {
        return Ok(None);
    };
    let text = render(
        &model_label(&session.last_model, config_dir),
        Some(&session),
        config_dir,
        false,
    );
    let message = StopMessage {
        system_message: format!("\n{text}"),
    };
    serde_json::to_string(&message)
        .map(Some)
        .map_err(StatuslineError::Encode)
}

// A failure shows Claude Code the label it already knows and Codex nothing, since a bare string is not hook JSON
#[must_use]
pub fn run_statusline(input: &str, deps: &Dependencies<'_>) -> String {
    let body: Fields = serde_json::from_str(input).unwrap_or_default();
    let codex = text_field(&body, "hook_event_name") == CODEX_STOP_EVENT;
    let config_dir = non_empty(deps.environment, "CLAUDE_CONFIG_DIR")
        .map_or_else(|| deps.home_dir.join(CONFIG_DIRECTORY), PathBuf::from);
    let temp_root = ["TMPDIR", "TEMP", "TMP"]
        .iter()
        .find_map(|name| non_empty(deps.environment, name))
        .map_or_else(|| deps.temp_dir.to_path_buf(), PathBuf::from);
    let cache_dir = temp_root.join(format!("{CACHE_DIRECTORY_PREFIX}{}", deps.owner_uid));
    let outcome = if codex {
        codex_message(&body, deps, &config_dir)
    } else {
        claude_line(&body, deps, &config_dir, &cache_dir)
    };
    if let Ok(Some(text)) = outcome {
        return text;
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
