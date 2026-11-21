mod fixtures;
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::sections::key_event_sinks::Settings;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_secret_managers::secret_contracts::options::SecretTags;
use betterlite_server::key_event_sinks::email::{CreatedKey, LegacyEmail};
use betterlite_server::key_event_sinks::ports::TeamDirectory;
use betterlite_server::key_event_sinks::secrets::SecretSink;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;
use tokio_util::task::TaskTracker;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use self::fixtures::{Board, BoardMailer, Levels, ScriptedManager, ScriptedTeams};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    settings: SinkSettings,
    client: bool,
    runtime: Runtime,
    // team_id to its metadata; a team missing here fails its lookup
    teams: BTreeMap<String, JsonData>,
    secrets: BTreeMap<String, String>,
    failing: BTreeSet<String>,
    alerting: Vec<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SinkSettings {
    store_virtual_keys: Option<bool>,
    prefix_for_stored_virtual_keys: String,
    description: Option<String>,
    tags: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Runtime {
    Absent,
    NoDatabase,
    Ready,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Step {
    Secret(SecretStep),
    Email(EmailStep),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum SecretStep {
    Store {
        secret_name: String,
        secret_token: SecretString,
        team_id: Option<String>,
    },
    Rotate {
        current_secret_name: String,
        new_secret_name: String,
        new_secret_value: SecretString,
        team_id: Option<String>,
    },
    Rename {
        current_secret_name: String,
        new_secret_name: String,
        team_id: Option<String>,
    },
    Delete {
        keys: Vec<LiteLLMVerificationToken>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum EmailStep {
    KeyCreatedEmail { response: JsonData },
    KeyRotatedEmail,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let board = Arc::new(Board::default());
        let _logging = set_default(tracing_subscriber::registry().with(Levels {
            board: Arc::clone(&board),
        }));
        let settings = Settings {
            store_virtual_keys: input.settings.store_virtual_keys == Some(true),
            prefix_for_stored_virtual_keys: input.settings.prefix_for_stored_virtual_keys.clone(),
            description: input.settings.description.clone(),
            tags: input.settings.tags.clone().map(SecretTags::Mapping),
            alerting: input.alerting.clone(),
        };
        let manager = ScriptedManager {
            board: Arc::clone(&board),
            secrets: input.secrets.clone(),
            failing: input.failing.clone(),
        };
        let teams = ScriptedTeams {
            board: Arc::clone(&board),
            metadata: input.teams.clone(),
            database: input.runtime == Runtime::Ready,
        };
        let directory: &dyn TeamDirectory = &teams;
        let sink = SecretSink {
            settings: &settings,
            manager: input.client.then_some(&manager),
            teams: (input.runtime != Runtime::Absent).then_some(directory),
        };
        let mut steps = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            let raised = runtime.block_on(perform(&sink, &board, step))?;
            steps.push(board.close_step(raised));
        }
        Ok(json!({"steps": steps}))
    }
}

async fn perform(
    sink: &SecretSink<'_, ScriptedManager>,
    board: &Arc<Board>,
    step: &Step,
) -> Result<bool, Box<HelperError>> {
    match step {
        Step::Secret(secret) => Ok(secret_step(sink, secret).await),
        Step::Email(email) => email_step(&sink.settings.alerting, board, email).await,
    }
}

async fn secret_step(sink: &SecretSink<'_, ScriptedManager>, step: &SecretStep) -> bool {
    match step {
        SecretStep::Store {
            secret_name,
            secret_token,
            team_id,
        } => sink
            .store(secret_name, secret_token, team_id.as_deref())
            .await
            .is_err(),
        SecretStep::Rotate {
            current_secret_name,
            new_secret_name,
            new_secret_value,
            team_id,
        } => sink
            .rotate(
                current_secret_name,
                new_secret_name,
                new_secret_value,
                team_id.as_deref(),
            )
            .await
            .is_err(),
        SecretStep::Rename {
            current_secret_name,
            new_secret_name,
            team_id,
        } => sink
            .rename(current_secret_name, new_secret_name, team_id.as_deref())
            .await
            .is_err(),
        SecretStep::Delete { keys } => sink.delete(keys).await.is_err(),
    }
}

async fn email_step(
    alerting: &[String],
    board: &Arc<Board>,
    step: &EmailStep,
) -> Result<bool, Box<HelperError>> {
    let tasks = TaskTracker::new();
    let email = LegacyEmail {
        alerting,
        mailer: Arc::new(BoardMailer {
            board: Arc::clone(board),
        }),
        tasks: &tasks,
    };
    match step {
        EmailStep::KeyCreatedEmail { response } => {
            let created: CreatedKey = serde_json::from_value(serde_json::to_value(response)?)?;
            let sent = email.send_key_created_email(created);
            tasks.close();
            tasks.wait().await;
            Ok(sent.is_err())
        }
        EmailStep::KeyRotatedEmail => {
            email.send_key_rotated_email();
            Ok(false)
        }
    }
}
