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
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Store {
        secret_name: String,
        secret_token: String,
        team_id: Option<String>,
    },
    Rotate {
        current_secret_name: String,
        new_secret_name: String,
        new_secret_value: String,
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
    KeyCreatedEmail {
        response: JsonData,
    },
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
            steps.push(json!({"raised": raised, "effects": board.take_effects(), "logs": board.take_logs()}));
        }
        Ok(json!({"steps": steps}))
    }
}

async fn perform(
    sink: &SecretSink<'_, ScriptedManager>,
    board: &Arc<Board>,
    step: &Step,
) -> Result<bool, Box<HelperError>> {
    let tasks = TaskTracker::new();
    let mailer = Arc::new(BoardMailer {
        board: Arc::clone(board),
    });
    let email = LegacyEmail {
        alerting: &sink.settings.alerting,
        mailer,
        tasks: &tasks,
    };
    let raised = match step {
        Step::Store {
            secret_name,
            secret_token,
            team_id,
        } => sink
            .store(
                secret_name,
                &SecretString::from(secret_token.as_str()),
                team_id.as_deref(),
            )
            .await
            .is_err(),
        Step::Rotate {
            current_secret_name,
            new_secret_name,
            new_secret_value,
            team_id,
        } => {
            let value = SecretString::from(new_secret_value.as_str());
            sink.rotate(
                current_secret_name,
                new_secret_name,
                &value,
                team_id.as_deref(),
            )
            .await
            .is_err()
        }
        Step::Rename {
            current_secret_name,
            new_secret_name,
            team_id,
        } => sink
            .rename(current_secret_name, new_secret_name, team_id.as_deref())
            .await
            .is_err(),
        Step::Delete { keys } => sink.delete(keys).await.is_err(),
        Step::KeyCreatedEmail { response } => {
            let created: CreatedKey = serde_json::from_value(serde_json::to_value(response)?)?;
            let sent = email.send_key_created_email(created);
            tasks.close();
            tasks.wait().await;
            sent.is_err()
        }
        Step::KeyRotatedEmail => {
            email.send_key_rotated_email();
            false
        }
    };
    Ok(raised)
}
