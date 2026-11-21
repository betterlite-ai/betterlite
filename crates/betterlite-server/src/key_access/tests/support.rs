use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::user_roles::UserRole;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::key_access::Settings;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::team_rows::team::TeamBase;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::key_access::service_account::TeamDirectory;
use crate::key_fields::decode::TypedValue;
use crate::key_fields::request::GenerateKeyRequest;
use crate::team_access::caller::TeamCaller;

pub(super) const TEAM: &str = r#"{"team_id":"team-1","members_with_roles":[{"user_id":"admin-1","role":"admin"},{"user_id":"member-1","role":"user"}],"team_member_permissions":["/key/generate"]}"#;

pub(super) fn team(text: &str) -> TeamBase<String> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn request(text: &str) -> GenerateKeyRequest {
    GenerateKeyRequest::read(&TypedValue {
        json: serde_json::from_str(text).unwrap(),
    })
    .unwrap()
}

pub(super) fn caller(user_id: Option<&str>, user_role: Option<UserRole>) -> TeamCaller {
    TeamCaller {
        user_id: user_id.map(str::to_owned),
        user_role,
        team_id: None,
        team_service_account: false,
    }
}

pub(super) fn member(user_id: &str) -> TeamCaller {
    caller(Some(user_id), Some(UserRole::InternalUser))
}

pub(super) fn settings(source: &str) -> Settings {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .key_access
}

// answers from a fixed team list and records each id it is asked about
#[derive(Default)]
pub(super) struct ListedTeams {
    pub(super) teams: Vec<&'static str>,
    pub(super) asked: Mutex<Vec<String>>,
}

#[async_trait]
impl TeamDirectory for ListedTeams {
    async fn team_exists(&self, team_id: &str) -> Result<bool, TeamQueryError> {
        self.asked.lock().unwrap().push(team_id.to_owned());
        Ok(self.teams.contains(&team_id))
    }
}

#[derive(Clone, Default)]
pub(super) struct Warnings(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() == Level::WARN {
            let mut message = Message::default();
            event.record(&mut message);
            self.0.lock().unwrap().push(message.0);
        }
    }
}

pub(super) fn warned<T, Run>(run: Run) -> (T, Vec<String>)
where
    Run: FnOnce() -> T,
{
    let warnings = Warnings::default();
    let value = with_default(tracing_subscriber::registry().with(warnings.clone()), run);
    let written = warnings.0.lock().unwrap().clone();
    (value, written)
}
