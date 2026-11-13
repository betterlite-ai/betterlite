use std::collections::BTreeMap;

use betterlite_db::database_reencryption::MigrationError;
use betterlite_db::database_reencryption::outcome::{
    MigrationFailure, MigrationOutcome, NothingToMigrate, describe_outcome,
};
use betterlite_db::database_reencryption::traversal::replace_ciphertexts;
use betterlite_db::encryption_scan::decode_migration_json;
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Replace {
                value,
                replacements,
                depth,
                max_depth,
            } => run_replace(value, replacements, *depth, *max_depth),
            Input::Outcome { outcome } => {
                Ok(Value::String(describe_outcome(&outcome.to_outcome())))
            }
        }
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Replace {
        value: Value,
        replacements: BTreeMap<String, Option<String>>,
        depth: i64,
        max_depth: i64,
    },
    Outcome {
        outcome: OutcomeInput,
    },
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutcomeInput {
    kind: OutcomeKind,
    migrated: usize,
    remaining: usize,
    error_class: String,
    error_message: String,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutcomeKind {
    Salt,
    NoDatabase,
    Nothing,
    Migrated,
    Failed,
}

impl OutcomeInput {
    fn to_outcome(&self) -> MigrationOutcome {
        match self.kind {
            OutcomeKind::Salt => {
                MigrationOutcome::Nothing(NothingToMigrate::SaltKeyEncryptsStoredValues)
            }
            OutcomeKind::NoDatabase => MigrationOutcome::Nothing(NothingToMigrate::NoDatabase),
            OutcomeKind::Nothing => {
                MigrationOutcome::Nothing(NothingToMigrate::NothingLeftToMigrate)
            }
            OutcomeKind::Migrated => MigrationOutcome::Migrated {
                migrated: self.migrated,
                remaining: self.remaining,
            },
            OutcomeKind::Failed => MigrationOutcome::Failed(MigrationFailure {
                cause: MigrationError::Reported {
                    message: self.error_message.clone(),
                },
                class: self.error_class.clone(),
                message: self.error_message.clone(),
            }),
        }
    }
}

fn input_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn run_replace(
    value: &Value,
    replacements: &BTreeMap<String, Option<String>>,
    depth: i64,
    max_depth: i64,
) -> HelperResult {
    let text = serde_json::to_string(value).map_err(|error| input_error(error.to_string()))?;
    let stored =
        decode_migration_json(text.as_bytes()).map_err(|error| input_error(error.to_string()))?;
    let mut replacement =
        |text: &str| Ok::<_, MigrationError>(replacements.get(text).cloned().flatten());
    let (replaced, count) = replace_ciphertexts(stored, &mut replacement, depth, max_depth)
        .map_err(|error| input_error(error.to_string()))?;
    let rendered =
        serde_json::to_value(&replaced).map_err(|error| input_error(error.to_string()))?;
    Ok(serde_json::json!({"value": rendered, "count": count}))
}
