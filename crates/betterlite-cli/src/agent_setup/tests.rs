#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::net::TcpListener;
use std::path::Path;

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tempfile::TempDir;

use crate::agent_setup::connection::{KEEP_CURRENT_KEY, NEW_KEY};
use crate::agent_setup::error::AgentSetupError;
use crate::agent_setup::listing::{
    ListingRequest, MAX_LISTING_BYTES, connection_listing, fetch_listing, starting_model,
    validated_model,
};
use crate::agent_setup::pickers::{
    CLAUDE_MODEL_MESSAGE, CODEX_MODEL_MESSAGE, EMPTY_TARGETS_MESSAGE, KEEP_CLAUDE_DEFAULT,
    TARGET_EDIT_MESSAGE, TARGET_ROUTE_MESSAGE, pick_claude_model, pick_codex_model,
};
use crate::agent_setup::{MODEL_OPTION_HELP, pick_targets, resolve_credential};
use crate::agent_setup_profiles::error::AgentSetupProfilesError;
use crate::agent_setup_profiles::{Target, read_saved_setup, settings_path_for};
use crate::claude_settings::choices::{STARTING_MODEL_ROLE, StaticToken};
use crate::cli_context::CliContext;
use crate::codex_settings::error::CodexSettingsError;
use crate::codex_settings::version::CodexVersion;
use crate::pi::ListedModel;

mod support;
include!("tests/credential.rs");
include!("tests/listing.rs");
include!("tests/prepare.rs");
include!("tests/apply.rs");
include!("tests/pickers.rs");

use support::{
    APPLY, Answer, Call, EDIT, EDIT_WIZARD, FakeEnvironment, MockProxy, Reply, Run, SAFE_CODEX,
    ScriptedPrompter, WIZARD, context, implicit_context, listed, listing, save, saved_setup,
    secret, token_file_context,
};

fn claude_settings(environment: &FakeEnvironment) -> BTreeMap<String, JsonValue> {
    let path = settings_path_for(Target::Claude, environment).unwrap();
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn env_string(settings: &BTreeMap<String, JsonValue>, key: &str) -> Option<String> {
    let env = settings.get("env")?;
    env.get(key)?.as_str().map(str::to_owned)
}

fn codex_document(environment: &FakeEnvironment) -> toml_edit::DocumentMut {
    let path = settings_path_for(Target::Codex, environment).unwrap();
    fs::read_to_string(path).unwrap().parse().unwrap()
}

fn codex_string(document: &toml_edit::DocumentMut, key: &str) -> Option<String> {
    document
        .get(key)
        .and_then(toml_edit::Item::as_str)
        .map(str::to_owned)
}

fn token(value: &str) -> StaticToken {
    StaticToken {
        token: secret(value),
    }
}

fn lines(parts: &[String]) -> String {
    format!("{}\n", parts.join("\n"))
}
