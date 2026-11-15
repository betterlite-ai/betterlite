use std::collections::BTreeMap;
use std::io;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::master_key_boot::counting::{
    StoredSecretCounter, with_stored_secrets_counted,
};
use betterlite_auth::master_key_boot::enforce::{
    BootError, BootReporter, OverrideWarning, enforce_boot_verdict,
};
use betterlite_auth::master_key_boot::refusal::{
    GENERATE_MASTER_KEY_COMMAND, MASTER_KEY_ENV_VAR, OVERRIDE_ENV_VAR,
    PRINT_NEW_MASTER_KEY_COMMAND, ROTATION_DOCS_URL, render_refusal,
};
use betterlite_auth::master_key_boot::verdict::{
    BootInput, BootVerdict, DatabaseState, KeyOrigin, MASTER_KEY_SETTING, OVERRIDE_SETTING,
    OverrideFlags, Refusal, StoredSecretsMigration, UnsafeReason, boot_verdict,
};
use betterlite_db::database_reencryption::PREVIOUS_KEY_VARIABLE;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
struct DatabaseInput {
    salt_key_is_set: bool,
    is_configured: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    master_key: Option<String>,
    environment_master_key: Option<String>,
    general_settings: BTreeMap<String, Value>,
    config_file_path: Option<String>,
    override_env_is_on: bool,
    database: DatabaseInput,
    stored_secret_count: Option<usize>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        Ok(runtime.block_on(answer(input)))
    }
}

async fn answer(input: &Input) -> Value {
    let stored_counter = FixedCounter {
        count: input.stored_secret_count,
        asked_about: Mutex::new(Vec::new()),
    };
    let counted = with_stored_secrets_counted(verdict_for(input), &stored_counter).await;
    let mut reporter = Recorder::default();
    let outcome = enforce_boot_verdict(&counted, &mut reporter);
    let error_kind = match &outcome {
        Ok(()) => "none",
        Err(BootError::Refused { .. }) => "refused",
        Err(BootError::Announce { .. }) => "announce",
    };
    json!({
        "verdict": verdict_report(&counted),
        "counted": { "asked_about": stored_counter.asked() },
        "warnings": warning_report(&reporter.warnings),
        "announcements": announcement_report(&counted, &reporter.announcements),
        "error_kind": error_kind,
    })
}

fn verdict_for(input: &Input) -> BootVerdict {
    boot_verdict(BootInput {
        master_key: input.master_key.clone().map(SecretString::from),
        environment_master_key: input.environment_master_key.clone().map(SecretString::from),
        master_key_in_general_settings: input.general_settings.contains_key(MASTER_KEY_SETTING),
        config_file_path: input.config_file_path.clone(),
        override_flags: OverrideFlags {
            env_is_on: input.override_env_is_on,
            setting_is_on: input.general_settings.get(OVERRIDE_SETTING) == Some(&Value::Bool(true)),
        },
        database: DatabaseState {
            salt_key_is_set: input.database.salt_key_is_set,
            configured: input.database.is_configured,
        },
    })
}

fn verdict_report(verdict: &BootVerdict) -> Value {
    match verdict {
        BootVerdict::Refused(refusal) => refusal_report(refusal),
        BootVerdict::Safe => base_report("safe", None),
        BootVerdict::Allowed { reason } => base_report("allowed", Some(*reason)),
    }
}

fn base_report(kind: &str, reason: Option<UnsafeReason>) -> Value {
    json!({
        "kind": kind,
        "reason": reason.map(UnsafeReason::wire_name),
        "source": null,
        "config_file_path": null,
        "environment_variable_is_set": null,
        "migration": null,
        "rendered": null,
    })
}

fn refusal_report(refusal: &Refusal) -> Value {
    let (source, config_file_path) = match &refusal.origin {
        KeyOrigin::ConfigFile { path } => ("config_file", path.as_deref()),
        KeyOrigin::Environment => ("environment", None),
    };
    json!({
        "kind": "refused",
        "reason": refusal.reason.wire_name(),
        "source": source,
        "config_file_path": config_file_path,
        "environment_variable_is_set": refusal.environment_variable_is_set,
        "migration": refusal.migration.as_ref().map(migration_report),
        "rendered": rendered_report(&render_refusal(refusal)),
    })
}

fn migration_report(migration: &StoredSecretsMigration) -> Value {
    json!({
        "from_master_key": migration.from_master_key.expose_secret(),
        "encrypted_value_count": migration.encrypted_value_count,
    })
}

fn rendered_report(text: &str) -> Value {
    let placeholder = format!("os.environ/{MASTER_KEY_ENV_VAR}");
    let override_env = format!("{OVERRIDE_ENV_VAR}=true");
    let override_setting = format!("general_settings.{OVERRIDE_SETTING}: true");
    json!({
        "steps": text.lines().filter(|line| is_step_line(line)).count(),
        "save_command": text.contains(GENERATE_MASTER_KEY_COMMAND),
        "print_command": text.contains(PRINT_NEW_MASTER_KEY_COMMAND),
        "migration_line": migration_line(text),
        "rotation_docs": text.contains(ROTATION_DOCS_URL),
        "config_placeholder": text.contains(&placeholder),
        "override_env_hint": text.contains(&override_env),
        "override_setting_hint": text.contains(&override_setting),
        "override_hint_last": last_paragraph(text).starts_with("Local development only: set "),
    })
}

fn is_step_line(line: &str) -> bool {
    line.split_once(". ").is_some_and(|(prefix, _rest)| {
        !prefix.is_empty() && prefix.chars().all(|character| character.is_ascii_digit())
    })
}

fn migration_line(text: &str) -> Option<String> {
    let marker = format!("{PREVIOUS_KEY_VARIABLE}=");
    text.lines()
        .find(|line| line.contains(&marker))
        .map(|line| line.trim().to_owned())
}

fn last_paragraph(text: &str) -> &str {
    text.rsplit_once("\n\n")
        .map_or(text, |(_earlier, last)| last)
}

fn warning_report(warnings: &[OverrideWarning]) -> Value {
    let prefix = format!("{OVERRIDE_SETTING} is on, so ");
    Value::Array(
        warnings
            .iter()
            .map(|warning| {
                json!({
                    "setting": warning.setting,
                    "message_shaped": warning.message.starts_with(&prefix)
                        && warning.message.ends_with("Never run this outside local development."),
                })
            })
            .collect(),
    )
}

fn announcement_report(verdict: &BootVerdict, announced: &[String]) -> Value {
    let expected = match verdict {
        BootVerdict::Refused(refusal) => {
            let rendered = render_refusal(refusal);
            format!("\n{rendered}\n\n")
        }
        BootVerdict::Safe | BootVerdict::Allowed { .. } => String::new(),
    };
    Value::Array(
        announced
            .iter()
            .map(|text| json!({ "matches_rendered": *text == expected }))
            .collect(),
    )
}

#[derive(Debug)]
struct FixedCounter {
    count: Option<usize>,
    asked_about: Mutex<Vec<String>>,
}

impl FixedCounter {
    fn asked(&self) -> Vec<String> {
        self.asked_about
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl StoredSecretCounter for FixedCounter {
    async fn count_values_encrypted_with(&self, signing_key: &SecretString) -> Option<usize> {
        self.asked_about
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(signing_key.expose_secret().to_owned());
        self.count
    }
}

#[derive(Debug, Default)]
struct Recorder {
    warnings: Vec<OverrideWarning>,
    announcements: Vec<String>,
}

impl BootReporter for Recorder {
    fn warn(&mut self, warning: OverrideWarning) {
        self.warnings.push(warning);
    }

    fn announce(&mut self, message: &str) -> Result<(), io::Error> {
        self.announcements.push(message.to_owned());
        Ok(())
    }
}
