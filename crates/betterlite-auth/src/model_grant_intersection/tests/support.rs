use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use serde::de::DeserializeOwned;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::auth_cache::AuthCacheError;
use crate::auth_cache::payload::PayloadError;
use crate::auth_identity::errors::AuthError as IdentityError;
use crate::auth_org_lookup::error::OrgLookupError;
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_user_lookup::error::{UserLookupError, UserReadFailure};
use crate::end_user_registry::errors::RegistryError;
use crate::membership_load::error::MembershipLoadError;
use crate::model_grant_intersection::sources::{GrantSources, ServingFailure, ServingGroups};
use crate::model_grant_intersection::token::GrantToken;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Effect {
    Team(String),
    User(String),
    Project(String),
    Org(String),
    Membership {
        user_id: String,
        team_id: String,
    },
    Registry,
    Serving {
        model: String,
        team_id: Option<String>,
    },
}

type ServingMap = BTreeMap<String, BTreeSet<String>>;

pub(super) struct World {
    pub(super) team: Mutex<Option<Result<LiteLLMTeamTableCachedObj, TeamLookupError>>>,
    pub(super) user: Mutex<Option<Result<Option<LiteLLMUserTable>, UserLookupError>>>,
    pub(super) project: Mutex<Option<Result<Option<CachedProject>, OrgLookupError>>>,
    pub(super) org: Mutex<Option<Result<Option<OrganizationTable>, OrgLookupError>>>,
    pub(super) membership:
        Mutex<Option<Result<Option<LiteLLMTeamMembership>, MembershipLoadError>>>,
    pub(super) registry: Mutex<Option<Result<Option<BTreeSet<String>>, RegistryError>>>,
    pub(super) serving: Mutex<Option<Result<ServingMap, ()>>>,
    effects: Mutex<Vec<Effect>>,
}

impl World {
    pub(super) const fn new() -> Self {
        Self {
            team: Mutex::new(None),
            user: Mutex::new(None),
            project: Mutex::new(None),
            org: Mutex::new(None),
            membership: Mutex::new(None),
            registry: Mutex::new(None),
            serving: Mutex::new(None),
            effects: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn with_team(
        self,
        answer: Result<LiteLLMTeamTableCachedObj, TeamLookupError>,
    ) -> Self {
        *self.team.lock().unwrap() = Some(answer);
        self
    }

    pub(super) fn with_user(
        self,
        answer: Result<Option<LiteLLMUserTable>, UserLookupError>,
    ) -> Self {
        *self.user.lock().unwrap() = Some(answer);
        self
    }

    pub(super) fn with_project(
        self,
        answer: Result<Option<CachedProject>, OrgLookupError>,
    ) -> Self {
        *self.project.lock().unwrap() = Some(answer);
        self
    }

    pub(super) fn with_org(
        self,
        answer: Result<Option<OrganizationTable>, OrgLookupError>,
    ) -> Self {
        *self.org.lock().unwrap() = Some(answer);
        self
    }

    pub(super) fn with_membership(
        self,
        answer: Result<Option<LiteLLMTeamMembership>, MembershipLoadError>,
    ) -> Self {
        *self.membership.lock().unwrap() = Some(answer);
        self
    }

    pub(super) fn with_registry(
        self,
        answer: Result<Option<BTreeSet<String>>, RegistryError>,
    ) -> Self {
        *self.registry.lock().unwrap() = Some(answer);
        self
    }

    pub(super) fn with_serving(self, groups: &[(&str, &[&str])]) -> Self {
        let mut map = BTreeMap::new();
        for (model, served) in groups {
            map.insert((*model).to_owned(), group_set(served));
        }
        *self.serving.lock().unwrap() = Some(Ok(map));
        self
    }

    pub(super) fn with_broken_router(self) -> Self {
        *self.serving.lock().unwrap() = Some(Err(()));
        self
    }

    fn watched<Answer>(&self, effect: Effect, script: &Mutex<Option<Answer>>) -> Answer {
        self.effects.lock().expect("effects lock").push(effect);
        play(script)
    }

    pub(super) fn effects(&self) -> Vec<Effect> {
        self.effects.lock().expect("effects lock").clone()
    }
}

fn play<Answer>(script: &Mutex<Option<Answer>>) -> Answer {
    let mut pending = script.lock().expect("script lock");
    pending
        .take()
        .expect("a source ran that the test did not script")
}

#[async_trait]
impl GrantSources for World {
    async fn team_object(
        &self,
        team_id: &str,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        self.watched(Effect::Team(team_id.to_owned()), &self.team)
    }

    async fn user_object(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, UserLookupError> {
        self.watched(Effect::User(user_id.to_owned()), &self.user)
    }

    async fn project_object(
        &self,
        project_id: &str,
        _now: f64,
    ) -> Result<Option<CachedProject>, OrgLookupError> {
        self.watched(Effect::Project(project_id.to_owned()), &self.project)
    }

    async fn organization(
        &self,
        org_id: &str,
    ) -> Result<Option<OrganizationTable>, OrgLookupError> {
        self.watched(Effect::Org(org_id.to_owned()), &self.org)
    }

    async fn membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        self.effects.lock().unwrap().push(Effect::Membership {
            user_id: user_id.to_owned(),
            team_id: team_id.to_owned(),
        });
        play(&self.membership)
    }

    async fn budget_groups(&self) -> Result<Option<BTreeSet<String>>, RegistryError> {
        self.effects.lock().unwrap().push(Effect::Registry);
        play(&self.registry)
    }
}

#[async_trait]
impl ServingGroups for World {
    async fn groups_for(
        &self,
        model: &str,
        team_id: Option<&str>,
    ) -> Result<BTreeSet<String>, ServingFailure> {
        self.effects.lock().unwrap().push(Effect::Serving {
            model: model.to_owned(),
            team_id: team_id.map(str::to_owned),
        });
        let script = self.serving.lock().unwrap();
        match script.as_ref() {
            Some(Ok(map)) => Ok(map.get(model).cloned().unwrap_or_default()),
            Some(Err(())) => {
                let failure: Box<dyn Error + Send + Sync> =
                    Box::new(IoError::other("the access group store is down"));
                Err(ServingFailure::from(failure))
            }
            None => panic!("the router ran while the test did not script it"),
        }
    }
}

fn row<Row>(text: &str) -> Row
where
    Row: DeserializeOwned,
{
    serde_json::from_str(text).unwrap()
}

fn quoted(values: &[&str]) -> String {
    let quoted: Vec<String> = values.iter().map(|value| format!(r#""{value}""#)).collect();
    quoted.join(",")
}

pub(super) fn owned(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn group_set(values: &[&str]) -> BTreeSet<String> {
    let mut groups = BTreeSet::new();
    for value in values {
        groups.insert((*value).to_owned());
    }
    groups
}

pub(super) fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

pub(super) fn list(values: &[&str]) -> JsonData {
    let mut entries = Vec::new();
    for value in values {
        entries.push(JsonData::String((*value).to_owned()));
    }
    JsonData::Array(entries)
}

pub(super) fn auth_object(entries: &[(&str, JsonData)]) -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    for (key, value) in entries {
        fields.insert((*key).to_owned(), value.clone());
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(super) fn grant_token(entries: &[(&str, JsonData)]) -> GrantToken {
    let auth = auth_object(entries);
    GrantToken::of(&auth)
}

pub(super) fn team_row(
    team_id: &str,
    models: &[&str],
    org: Option<&str>,
) -> LiteLLMTeamTableCachedObj {
    let organization = org.map_or_else(String::new, |org_id| {
        format!(r#","organization_id":"{org_id}""#)
    });
    row(&format!(
        r#"{{"team_id":"{team_id}","models":[{}]{organization},"last_refreshed_at":1.0}}"#,
        quoted(models)
    ))
}

pub(super) fn user_row(user_id: &str, models: &[&str]) -> LiteLLMUserTable {
    row(&format!(
        r#"{{"user_id":"{user_id}","models":[{}]}}"#,
        quoted(models)
    ))
}

pub(super) fn project_row(project_id: &str, models: &[&str]) -> CachedProject {
    row(&format!(
        r#"{{"project_id":"{project_id}","models":[{}],"last_refreshed_at":1.0}}"#,
        quoted(models)
    ))
}

pub(super) fn organization_row(org_id: &str, models: &[&str]) -> OrganizationTable {
    row(&format!(
        r#"{{"organization_id":"{org_id}","budget_id":"b","created_by":"c","updated_by":"c","models":[{}]}}"#,
        quoted(models)
    ))
}

pub(super) fn membership_row(
    user_id: &str,
    team_id: &str,
    allowed: Option<&[&str]>,
) -> LiteLLMTeamMembership {
    allowed.map_or_else(
        || row(&format!(r#"{{"user_id":"{user_id}","team_id":"{team_id}"}}"#)),
        |models| {
            row(&format!(
                r#"{{"user_id":"{user_id}","team_id":"{team_id}","litellm_budget_table":{{"allowed_models":[{}]}}}}"#,
                quoted(models)
            ))
        },
    )
}

pub(super) fn unavailable() -> Box<dyn Error + Send + Sync> {
    Box::new(IoError::other("the database is down"))
}

pub(super) fn missing_user() -> UserLookupError {
    UserLookupError::NotFound(IdentityError::UserNotFound {
        user_id: "u1".to_owned(),
    })
}

pub(super) fn user_read_failure() -> UserLookupError {
    UserLookupError::UserMissing(UserReadFailure {
        user_id: "u1".to_owned(),
        cause: unavailable(),
    })
}

pub(super) fn cache_failure() -> AuthCacheError {
    AuthCacheError::Payload(Box::new(PayloadError::Json(serde_json::Error::io(
        IoError::other("scripted cache failure"),
    ))))
}

pub(super) fn registry_failure() -> RegistryError {
    RegistryError::CacheRead(cache_failure())
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
