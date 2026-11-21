#![cfg(test)]
use std::collections::BTreeMap;
use std::future::ready;
use std::sync::Mutex;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use rstest::rstest;

use super::AgentCallerLoaders;
use crate::agent_caller::error::AgentCallerError;
use crate::agent_caller::reads::CallerReads;
use crate::agent_policy::caller::AgentCaller;
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_user_lookup::error::UserLookupError;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

type TeamRead = Result<LiteLLMTeamTableCachedObj, TeamLookupError>;

struct FakeCallerReads {
    teams: BTreeMap<String, LiteLLMTeamTableCachedObj>,
    users: BTreeMap<String, Option<LiteLLMUserTable>>,
    asked: Mutex<Vec<String>>,
}

impl CallerReads for FakeCallerReads {
    fn team(&self, team_id: &str) -> impl Future<Output = TeamRead> + Send {
        self.asked.lock().unwrap().push(format!("team:{team_id}"));
        let missing = TeamLookupError::TeamNotFound {
            team_id: team_id.to_owned(),
        };
        ready(self.teams.get(team_id).cloned().ok_or(missing))
    }

    fn user(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, UserLookupError>> + Send {
        self.asked.lock().unwrap().push(format!("user:{user_id}"));
        // a stored None is a read that fails, a missing key is a user without a row
        ready(match self.users.get(user_id) {
            Some(Some(row)) => Ok(Some(row.clone())),
            Some(None) => Err(UserLookupError::NoDatabase),
            None => Ok(None),
        })
    }
}

fn team(team_id: &str) -> LiteLLMTeamTableCachedObj {
    let row: JsonData = serde_json::from_str(&format!(
        r#"{{"team_id": "{team_id}", "models": ["gpt-4o"]}}"#
    ))
    .unwrap();
    LiteLLMTeamTableCachedObj::try_from(row).unwrap()
}

fn user(user_id: &str) -> LiteLLMUserTable {
    serde_json::from_str(&format!(r#"{{"user_id": "{user_id}"}}"#)).unwrap()
}

fn loaders() -> AgentCallerLoaders<FakeCallerReads> {
    AgentCallerLoaders {
        reads: FakeCallerReads {
            teams: BTreeMap::from([("humans".to_owned(), team("humans"))]),
            users: BTreeMap::from([
                ("human".to_owned(), Some(user("human"))),
                ("broken".to_owned(), None),
            ]),
            asked: Mutex::new(Vec::new()),
        },
    }
}

fn key(user_id: Option<&str>, team_id: Option<&str>, forwarded: bool) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(r#"{"agent_id": "agent-1", "user_id": "owner", "team_id": "owners"}"#)
            .unwrap();
    let mut auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap();
    if forwarded {
        auth.extra.agent_caller = Some(AgentCaller {
            user_id: user_id.map(str::to_owned),
            team_id: team_id.map(str::to_owned),
        });
    }
    auth
}

fn asked(loaders: &AgentCallerLoaders<FakeCallerReads>) -> Vec<String> {
    loaders.reads.asked.lock().unwrap().clone()
}

#[rstest]
#[case::no_caller(key(None, None, false))]
#[case::caller_without_a_team(key(Some("human"), None, true))]
#[tokio::test]
async fn no_caller_team_id_loads_no_team(#[case] auth: UserAPIKeyAuth) {
    let loaders = loaders();
    assert!(
        loaders
            .load_agent_caller_team(&auth)
            .await
            .unwrap()
            .is_none()
    );
    assert!(asked(&loaders).is_empty());
}

#[tokio::test]
async fn the_caller_team_is_read_by_its_id() {
    let loaders = loaders();
    let auth = key(Some("human"), Some("humans"), true);
    let loaded = loaders
        .load_agent_caller_team(&auth)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.team.fields.base.team_id, "humans");
    assert_eq!(asked(&loaders), ["team:humans"]);
}

#[tokio::test]
async fn an_unreadable_caller_team_is_an_error_not_an_absent_team() {
    let loaders = loaders();
    let auth = key(None, Some("ghosts"), true);
    let error = loaders.load_agent_caller_team(&auth).await.unwrap_err();
    assert!(matches!(
        &error,
        AgentCallerError::Team { team_id, source }
            if team_id == "ghosts" && matches!(**source, TeamLookupError::TeamNotFound { .. })
    ));
}

#[rstest]
#[case::no_caller(key(None, None, false))]
#[case::caller_without_a_user(key(None, Some("humans"), true))]
#[tokio::test]
async fn no_caller_user_id_loads_no_user(#[case] auth: UserAPIKeyAuth) {
    let loaders = loaders();
    assert!(
        loaders
            .load_agent_caller_user(&auth)
            .await
            .unwrap()
            .is_none()
    );
    assert!(asked(&loaders).is_empty());
}

#[tokio::test]
async fn the_caller_user_is_read_by_its_id() {
    let loaders = loaders();
    let auth = key(Some("human"), Some("humans"), true);
    let loaded = loaders
        .load_agent_caller_user(&auth)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.user_id, "human");
    assert_eq!(asked(&loaders), ["user:human"]);
}

#[tokio::test]
async fn a_caller_user_without_a_row_narrows_nothing() {
    let loaders = loaders();
    let auth = key(Some("nobody"), None, true);
    assert!(
        loaders
            .load_agent_caller_user(&auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(asked(&loaders), ["user:nobody"]);
}

#[tokio::test]
async fn a_failed_caller_user_read_is_returned() {
    let loaders = loaders();
    let auth = key(Some("broken"), None, true);
    let error = loaders.load_agent_caller_user(&auth).await.unwrap_err();
    assert!(matches!(
        &error,
        AgentCallerError::User { user_id, source }
            if user_id == "broken" && matches!(**source, UserLookupError::NoDatabase)
    ));
}
