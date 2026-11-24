#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_group_lookups::access_groups::{AccessGroupMap, AccessGroups};

use super::annotate::{populate_team_access, visible_models};
use super::compose::{
    ModelCreator, ScopeCaller, ScopeFuture, ScopeReads, TeamSelection, UserScope,
    accessible_models, non_admin_all_models,
};
use super::error::ScopeError;
use super::teams::{TeamModelLists, TeamScope};
use crate::model_visibility::testing::{Row, Snapshot};

mod postgres;

fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn team_ids(ids: &[&str]) -> Json {
    let texts: Vec<Json> = ids.iter().map(|id| Json::Text(String::from(*id))).collect();
    Json::List(texts)
}

fn team_list(teams: &[&str]) -> Vec<String> {
    teams.iter().map(|team| String::from(*team)).collect()
}

fn deployment(name: &str, info: Vec<(&str, Json)>) -> DeploymentSpec {
    DeploymentSpec::new(
        name.to_owned(),
        object(vec![("model", Json::Text("openai/gpt-4o".to_owned()))]),
        object(info),
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

fn identified(name: &str, id: &str) -> DeploymentSpec {
    deployment(name, vec![("id", Json::Text(id.to_owned()))])
}

fn team_row(name: &str, id: &str, team: &str) -> DeploymentSpec {
    deployment(
        name,
        vec![
            ("id", Json::Text(id.to_owned())),
            ("team_id", Json::Text(team.to_owned())),
        ],
    )
}

fn unnumbered(name: &str, info: Vec<(&str, Json)>) -> DeploymentSpec {
    DeploymentSpec {
        model_name: name.to_owned(),
        litellm_params: object(vec![("model", Json::Text("openai/gpt-4o".to_owned()))]),
        model_info: object(info),
        extra: JsonObject::new(),
    }
}

fn snapshot(rows: &[DeploymentSpec]) -> Snapshot {
    let parts: Vec<(String, JsonObject, Json)> = rows
        .iter()
        .map(|row| {
            (
                row.model_name.clone(),
                row.litellm_params.clone(),
                Json::Object(row.model_info.clone()),
            )
        })
        .collect();
    let declared: Vec<Row<'_>> = parts
        .iter()
        .map(|(name, params, info)| Row {
            model_name: name.as_str(),
            litellm_params: params,
            model_info: info,
        })
        .collect();
    Snapshot::build(&declared, &JsonObject::new()).unwrap()
}

fn selection_label(selection: &TeamSelection) -> String {
    match selection {
        TeamSelection::All => "teams:*".to_owned(),
        TeamSelection::Ids(ids) => format!("teams:{}", ids.join(",")),
    }
}

#[derive(Default)]
struct StubReads {
    creators: BTreeMap<String, Option<String>>,
    user: Option<UserScope>,
    user_fails: bool,
    teams: Vec<TeamScope>,
    teams_fail: bool,
    groups: AccessGroupMap,
    groups_fail: bool,
    calls: Mutex<Vec<String>>,
}

impl StubReads {
    fn note(&self, label: String) {
        let mut calls = self.calls.lock().unwrap();
        calls.push(label);
    }

    fn creator(&self, model_id: &str) -> Result<Option<ModelCreator>, ScopeError> {
        self.note(format!("model:{model_id}"));
        let creator = self.creators.get(model_id).map(|created_by| ModelCreator {
            created_by: created_by.clone(),
        });
        Ok(creator)
    }

    fn user_row(&self, user_id: &str) -> Result<Option<UserScope>, ScopeError> {
        self.note(format!("user:{user_id}"));
        if !self.user_fails {
            return Ok(self.user.clone());
        }
        let failure = std::io::Error::other("user read failed");
        Err(ScopeError::UserRead(Box::new(failure)))
    }

    fn team_rows(&self, selection: &TeamSelection) -> Result<Vec<TeamScope>, ScopeError> {
        self.note(selection_label(selection));
        if !self.teams_fail {
            return Ok(self.teams.clone());
        }
        let failure = std::io::Error::other("team read failed");
        Err(ScopeError::TeamRead(Box::new(failure)))
    }

    fn group_rows(&self, access_group_ids: &[String]) -> Result<AccessGroupMap, ScopeError> {
        self.note(format!("groups:{}", access_group_ids.join(",")));
        if !self.groups_fail {
            return Ok(self.groups.clone());
        }
        let failure = std::io::Error::other("access group read failed");
        Err(ScopeError::AccessGroupRead(Box::new(failure)))
    }
}

impl ScopeReads for StubReads {
    fn model_creator<'a>(
        &'a self,
        model_id: &'a str,
    ) -> ScopeFuture<'a, Result<Option<ModelCreator>, ScopeError>> {
        Box::pin(async move { self.creator(model_id) })
    }

    fn user_scope<'a>(
        &'a self,
        user_id: &'a str,
    ) -> ScopeFuture<'a, Result<Option<UserScope>, ScopeError>> {
        Box::pin(async move { self.user_row(user_id) })
    }

    fn team_scopes<'a>(
        &'a self,
        selection: &'a TeamSelection,
    ) -> ScopeFuture<'a, Result<Vec<TeamScope>, ScopeError>> {
        Box::pin(async move { self.team_rows(selection) })
    }

    fn access_group_models<'a>(
        &'a self,
        access_group_ids: &'a [String],
    ) -> ScopeFuture<'a, Result<AccessGroupMap, ScopeError>> {
        Box::pin(async move { self.group_rows(access_group_ids) })
    }
}

fn caller(user_id: Option<&str>, key_models: &[&str]) -> ScopeCaller {
    ScopeCaller {
        user_id: user_id.map(str::to_owned),
        key_models: key_models.iter().map(|name| String::from(*name)).collect(),
        admin_view: false,
    }
}

fn restricted(team_id: &str, models: &[&str], groups: &[&str]) -> TeamScope {
    TeamScope {
        team_id: team_id.to_owned(),
        models: models.iter().map(|name| String::from(*name)).collect(),
        access_group_ids: groups.iter().map(|id| String::from(*id)).collect(),
    }
}

fn assert_annotations(model: &DeploymentSpec, direct: bool, teams: Option<&[&str]>) {
    assert_eq!(
        model.model_info.get("direct_access"),
        Some(&Json::Bool(direct))
    );
    match teams {
        Some(ids) => assert_eq!(
            model.model_info.get("access_via_team_ids"),
            Some(&team_ids(ids))
        ),
        None => assert!(model.model_info.get("access_via_team_ids").is_none()),
    }
}

fn assert_calls(store: &StubReads, expected: &[&str]) {
    let wanted: Vec<String> = expected.iter().map(|call| (*call).to_owned()).collect();
    assert_eq!(*store.calls.lock().unwrap(), wanted);
}

fn ids(models: &[DeploymentSpec]) -> Vec<Option<&str>> {
    models.iter().map(|model| model.id()).collect()
}

#[tokio::test]
async fn admin_annotations() {
    let rows = vec![
        identified("gpt-4o", "global-1"),
        team_row("team-gpt", "team-1", "team-alpha"),
        team_row("other-gpt", "team-2", "team-beta"),
    ];
    let snapshot = snapshot(&rows);
    let mut store = StubReads::default();
    store.teams = vec![restricted("team-alpha", &[], &[])];
    let groups = AccessGroups::default();
    let admin = ScopeCaller {
        admin_view: true,
        ..caller(Some("admin"), &[])
    };
    let annotated = accessible_models(rows, true, &admin, &store, &snapshot.lookup(), &groups)
        .await
        .unwrap();
    assert_eq!(ids(&annotated), vec![Some("global-1"), Some("team-1")]);
    assert_annotations(&annotated[0], true, Some(&["team-alpha"]));
    assert_annotations(&annotated[1], false, Some(&["team-alpha"]));
    assert_calls(&store, &["teams:*"]);
}

#[tokio::test]
async fn missing_user_annotations() {
    let rows = vec![
        deployment(
            "gpt-4o",
            vec![
                ("id", Json::Text("g1".to_owned())),
                ("access_via_team_ids", team_ids(&["team-old"])),
            ],
        ),
        identified("gpt-4o-mini", "g2"),
    ];
    let snapshot = snapshot(&rows);
    let store = StubReads::default();
    let groups = AccessGroups::default();
    let annotated = accessible_models(
        rows,
        false,
        &caller(Some("ghost"), &[]),
        &store,
        &snapshot.lookup(),
        &groups,
    )
    .await
    .unwrap();
    assert_eq!(annotated.len(), 2);
    assert_annotations(&annotated[0], false, Some(&["team-old"]));
    assert_annotations(&annotated[1], false, None);
    let visible = visible_models(annotated);
    assert_eq!(ids(&visible), vec![Some("g1")]);
    assert_calls(&store, &["user:ghost"]);
}

#[test]
fn missing_id_preserves_fields() {
    let mut rows = vec![DeploymentSpec {
        model_name: "gpt-4o".to_owned(),
        litellm_params: object(vec![("model", Json::Text("openai/gpt-4o".to_owned()))]),
        model_info: object(vec![
            ("team_id", Json::Text("team-alpha".to_owned())),
            ("access_via_team_ids", team_ids(&["kept-team"])),
            ("custom", Json::Text("kept".to_owned())),
        ]),
        extra: JsonObject::new(),
    }];
    let mut team_models = TeamModelLists::new();
    team_models.insert("some-id".to_owned(), team_list(&["team-alpha"]));
    populate_team_access(&mut rows, Some(&team_models), &["some-id".to_owned()]);
    assert!(rows[0].model_info.get("direct_access").is_none());
    assert_eq!(
        rows[0].model_info.get("access_via_team_ids"),
        Some(&team_ids(&["kept-team"]))
    );
    assert_eq!(
        rows[0].model_info.get("custom"),
        Some(&Json::Text("kept".to_owned()))
    );
    assert_eq!(visible_models(rows).len(), 1);
}

#[test]
fn ineligible_annotation_preserved() {
    let mut rows = vec![deployment(
        "team-gpt",
        vec![
            ("id", Json::Text("t2".to_owned())),
            ("team_id", Json::Text("team-beta".to_owned())),
            ("access_via_team_ids", team_ids(&["stale"])),
        ],
    )];
    let mut team_models = TeamModelLists::new();
    team_models.insert("t2".to_owned(), team_list(&["team-alpha"]));
    populate_team_access(&mut rows, Some(&team_models), &[]);
    assert_annotations(&rows[0], false, Some(&["stale"]));
}

#[test]
fn exact_team_annotation_gate() {
    let mut rows = vec![
        team_row("alpha-gpt", "a", "team-alpha"),
        team_row("short-gpt", "b", "team"),
        team_row("long-gpt", "c", "team-alphabet"),
        identified("global-gpt", "d"),
    ];
    let mut team_models = TeamModelLists::new();
    team_models.insert("a".to_owned(), team_list(&["team-alpha"]));
    team_models.insert("b".to_owned(), team_list(&["team-alpha"]));
    team_models.insert("c".to_owned(), team_list(&["team-alpha"]));
    populate_team_access(&mut rows, Some(&team_models), &[]);
    assert_annotations(&rows[0], false, Some(&["team-alpha"]));
    assert_annotations(&rows[1], false, None);
    assert_annotations(&rows[2], false, None);
    assert_annotations(&rows[3], false, Some(&[]));
}

#[tokio::test]
async fn creator_matches_anonymous_row() {
    let rows = vec![identified("gpt-4o", "d3")];
    let snapshot = snapshot(&rows);
    let mut store = StubReads::default();
    store.creators.insert("d3".to_owned(), None);
    let kept = non_admin_all_models(rows, &caller(None, &[]), Some(&store), &snapshot.lookup())
        .await
        .unwrap();
    assert_eq!(ids(&kept), vec![Some("d3")]);
    assert_calls(&store, &["model:d3"]);
}

#[tokio::test]
async fn annotation_team_read_failure() {
    let rows = vec![identified("gpt-4o", "g1")];
    let snapshot = snapshot(&rows);
    let groups = AccessGroups::default();
    let mut store = StubReads::default();
    store.user = Some(UserScope {
        teams: team_list(&["team-alpha"]),
        models: Vec::new(),
    });
    store.teams_fail = true;
    let error = accessible_models(
        rows,
        false,
        &caller(Some("alice"), &[]),
        &store,
        &snapshot.lookup(),
        &groups,
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(matches!(error, ScopeError::TeamRead(_)));
}

#[tokio::test]
async fn annotation_group_read_failure() {
    let rows = vec![identified("gpt-4o", "g1")];
    let snapshot = snapshot(&rows);
    let groups = AccessGroups::default();
    let mut store = StubReads::default();
    store.user = Some(UserScope {
        teams: team_list(&["team-alpha"]),
        models: Vec::new(),
    });
    store.teams = vec![restricted("team-alpha", &["gpt-4o"], &["ag-1"])];
    store.groups_fail = true;
    let error = accessible_models(
        rows,
        false,
        &caller(Some("alice"), &[]),
        &store,
        &snapshot.lookup(),
        &groups,
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(matches!(error, ScopeError::AccessGroupRead(_)));
}
