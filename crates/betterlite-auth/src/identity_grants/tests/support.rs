use std::error::Error;
use std::future;
use std::io::Error as IoError;
use std::mem;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_team_lookup::lookup::{TeamRequest, TeamSource};
use crate::auth_user_lookup::error::UserLookupError;
use crate::auth_user_lookup::lookup::UserLookupRequest;
use crate::identity_grants::loaders::{MembershipLoader, TeamLoader, UserLoader};
use crate::identity_grants::records::UserQuery;
use crate::identity_grants::resolver::GrantResolver;
use crate::membership_load::error::MembershipLoadError;

// Each loader answers once; Unused fails a test whose loader ran when it should not have.
pub(super) enum Script<Row, Failure> {
    Answer(Result<Row, Failure>),
    Pending,
    Unused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Effect {
    User {
        user_id: Option<String>,
        sso_user_id: Option<String>,
        user_email: Option<String>,
        upsert: bool,
        check_db_only: Option<bool>,
    },
    Membership {
        user_id: String,
        team_id: String,
    },
    Team {
        team_id: String,
        source: TeamSource,
        upsert: bool,
    },
}

pub(super) struct World {
    pub(super) user: Mutex<Script<Option<LiteLLMUserTable>, UserLookupError>>,
    pub(super) membership: Mutex<Script<Option<LiteLLMTeamMembership>, MembershipLoadError>>,
    pub(super) team: Mutex<Script<LiteLLMTeamTableCachedObj, TeamLookupError>>,
    effects: Mutex<Vec<Effect>>,
}

impl World {
    pub(super) const fn new() -> Self {
        Self {
            user: Mutex::new(Script::Unused),
            membership: Mutex::new(Script::Unused),
            team: Mutex::new(Script::Unused),
            effects: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn with_user(
        self,
        answer: Result<Option<LiteLLMUserTable>, UserLookupError>,
    ) -> Self {
        *self.user.lock().unwrap() = Script::Answer(answer);
        self
    }

    pub(super) fn with_membership(
        self,
        answer: Result<Option<LiteLLMTeamMembership>, MembershipLoadError>,
    ) -> Self {
        *self.membership.lock().unwrap() = Script::Answer(answer);
        self
    }

    pub(super) fn with_team(
        self,
        answer: Result<LiteLLMTeamTableCachedObj, TeamLookupError>,
    ) -> Self {
        *self.team.lock().unwrap() = Script::Answer(answer);
        self
    }

    pub(super) fn effects(&self) -> Vec<Effect> {
        self.effects.lock().unwrap().clone()
    }

    pub(super) const fn resolver(&self) -> GrantResolver<'_, Self, Self, Self> {
        GrantResolver {
            users: self,
            teams: self,
            memberships: self,
        }
    }
}

async fn play<Row, Failure>(script: &Mutex<Script<Row, Failure>>) -> Result<Row, Failure> {
    let taken = mem::replace(&mut *script.lock().unwrap(), Script::Unused);
    match taken {
        Script::Answer(answer) => answer,
        Script::Pending => future::pending().await,
        Script::Unused => panic!("a loader ran that the test did not script"),
    }
}

#[async_trait]
impl UserLoader for World {
    async fn load_user(
        &self,
        request: UserLookupRequest<'_>,
    ) -> Result<Option<LiteLLMUserTable>, UserLookupError> {
        self.effects.lock().unwrap().push(Effect::User {
            user_id: request.user_id.map(str::to_owned),
            sso_user_id: request.sso_user_id.map(str::to_owned),
            user_email: request.user_email.map(str::to_owned),
            upsert: request.user_id_upsert,
            check_db_only: request.check_db_only,
        });
        play(&self.user).await
    }
}

#[async_trait]
impl MembershipLoader for World {
    async fn load_membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        self.effects.lock().unwrap().push(Effect::Membership {
            user_id: user_id.to_owned(),
            team_id: team_id.to_owned(),
        });
        play(&self.membership).await
    }
}

#[async_trait]
impl TeamLoader for World {
    async fn load_team(
        &self,
        request: TeamRequest<'_>,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        self.effects.lock().unwrap().push(Effect::Team {
            team_id: request.team_id.to_owned(),
            source: request.source,
            upsert: request.team_id_upsert,
        });
        play(&self.team).await
    }
}

pub(super) fn query(user_id: Option<&str>) -> UserQuery {
    UserQuery {
        user_id: user_id.map(str::to_owned),
        user_email: None,
        sso_user_id: None,
        upsert: false,
    }
}

pub(super) fn user(user_id: &str, teams: &[&str]) -> LiteLLMUserTable {
    let mut row: LiteLLMUserTable = serde_json::from_str(r#"{"user_id": ""}"#).unwrap();
    user_id.clone_into(&mut row.user_id);
    row.teams = teams.iter().map(|&team_id| team_id.to_owned()).collect();
    row
}

pub(super) fn team(team_id: &str) -> LiteLLMTeamTableCachedObj {
    serde_json::from_str(&format!(r#"{{"team_id": "{team_id}"}}"#)).unwrap()
}

pub(super) fn membership(user_id: &str, team_id: &str) -> LiteLLMTeamMembership {
    let mut row: LiteLLMTeamMembership =
        serde_json::from_str(r#"{"user_id": "", "team_id": ""}"#).unwrap();
    user_id.clone_into(&mut row.user_id);
    team_id.clone_into(&mut row.team_id);
    row
}

pub(super) fn unavailable() -> Box<dyn Error + Send + Sync> {
    Box::new(IoError::other("the database is down"))
}

// Every event as its level and Debug text, which carries the message and each recorded field.
#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<String>>>);

impl Logs {
    pub(super) fn install(&self) -> DefaultGuard {
        set_default(tracing_subscriber::registry().with(self.clone()))
    }

    pub(super) fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl<S: Subscriber> Layer<S> for Logs {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let level = event.metadata().level();
        self.0.lock().unwrap().push(format!("{level} {event:?}"));
    }
}
