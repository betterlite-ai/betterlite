#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use axum::http::StatusCode;
use betterlite_auth::auth_model_patterns::ALL_PROXY_MODELS;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_group_lookups::access_groups::{AccessGroupMap, AccessGroups};

use super::annotate::{populate_team_access, visible_models};
use super::compose::{
    ModelCreator, ScopeCaller, ScopeReads, TeamSelection, UserScope, non_admin_all_models,
    read_caller_rows, read_user_models,
};
use super::error::ScopeError;
use super::grants::direct_access_models;
use super::teams::{TeamModelLists, TeamModelMap, TeamScope, add_access_group_models};
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

// A stored row without an id: DeploymentSpec::new always assigns one, so the fixture strips it again.
fn unnumbered(name: &str) -> DeploymentSpec {
    let mut spec = deployment(name, Vec::new());
    spec.model_info.retain(|field, _| field.as_str() != "id");
    spec
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

    fn creator(&self, model_id: &str) -> Option<ModelCreator> {
        self.note(format!("model:{model_id}"));
        self.creators.get(model_id).map(|created_by| ModelCreator {
            created_by: created_by.clone(),
        })
    }

    fn user_row(&self, user_id: &str) -> Option<UserScope> {
        self.note(format!("user:{user_id}"));
        self.user.clone()
    }

    fn team_rows(&self, selection: &TeamSelection) -> Result<Vec<TeamScope>, ScopeError> {
        self.note(selection_label(selection));
        if !self.teams_fail {
            return Ok(self.teams.clone());
        }
        let failure = TeamQueryError::from(DbError::NoDbConnected);
        Err(ScopeError::TeamRead(Box::new(failure)))
    }

    fn group_rows(&self, access_group_ids: &[String]) -> Result<AccessGroupMap, ScopeError> {
        self.note(format!("groups:{}", access_group_ids.join(",")));
        if !self.groups_fail {
            return Ok(self.groups.clone());
        }
        let failure = TableError::from(DbError::NoDbConnected);
        Err(ScopeError::AccessGroupRead(Box::new(failure)))
    }
}

#[async_trait]
impl ScopeReads for StubReads {
    async fn model_creator(&self, model_id: &str) -> Result<Option<ModelCreator>, ScopeError> {
        Ok(self.creator(model_id))
    }

    async fn user_scope(&self, user_id: &str) -> Result<Option<UserScope>, ScopeError> {
        Ok(self.user_row(user_id))
    }

    async fn team_scopes(&self, selection: &TeamSelection) -> Result<Vec<TeamScope>, ScopeError> {
        self.team_rows(selection)
    }

    async fn access_group_models(
        &self,
        access_group_ids: &[String],
    ) -> Result<AccessGroupMap, ScopeError> {
        self.group_rows(access_group_ids)
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
    let store = StubReads {
        creators: BTreeMap::new(),
        user: None,
        teams: vec![restricted("team-alpha", &[], &[])],
        teams_fail: false,
        groups: AccessGroupMap::new(),
        groups_fail: false,
        calls: Mutex::new(Vec::new()),
    };
    let admin = ScopeCaller {
        admin_view: true,
        ..caller(Some("admin"), &[])
    };
    let reading = read_caller_rows(&admin, &store).await.unwrap();
    let lookup = snapshot(&rows);
    let annotated = admin
        .accessible_models(
            rows,
            true,
            reading,
            (&lookup.lookup(), &AccessGroups::default()),
        )
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
    let store = StubReads::default();
    let ghost = caller(Some("ghost"), &[]);
    let reading = read_caller_rows(&ghost, &store).await.unwrap();
    let lookup = snapshot(&rows);
    let annotated = ghost
        .accessible_models(
            rows,
            false,
            reading,
            (&lookup.lookup(), &AccessGroups::default()),
        )
        .unwrap();
    assert_eq!(annotated.len(), 2);
    assert_annotations(&annotated[0], false, Some(&["team-old"]));
    assert_annotations(&annotated[1], false, None);
    let visible = visible_models(annotated);
    assert_eq!(ids(&visible), vec![Some("g1")]);
    assert_calls(&store, &["user:ghost"]);
}

#[tokio::test]
async fn anonymous_annotations() {
    let rows = vec![identified("gpt-4o-mini", "a1")];
    let store = StubReads::default();
    let anonymous = caller(None, &[]);
    let reading = read_caller_rows(&anonymous, &store).await.unwrap();
    let lookup = snapshot(&rows);
    let populated = anonymous
        .accessible_models(
            rows,
            false,
            reading,
            (&lookup.lookup(), &AccessGroups::default()),
        )
        .unwrap();
    assert_annotations(&populated[0], false, None);
    assert_calls(&store, &[]);
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
    let mut store = StubReads::default();
    store.creators.insert("d3".to_owned(), None);
    let anonymous = caller(None, &[]);
    let stored = read_user_models(rows.clone(), &anonymous, Some(&store))
        .await
        .unwrap();
    let lookup = snapshot(&rows);
    let kept = non_admin_all_models(stored, &lookup.lookup()).unwrap();
    assert_eq!(ids(&kept), vec![Some("d3")]);
    assert_calls(&store, &["model:d3"]);
}

#[tokio::test]
async fn annotation_team_read_failure() {
    let store = StubReads {
        creators: BTreeMap::new(),
        user: Some(UserScope {
            teams: team_list(&["team-alpha"]),
            models: Vec::new(),
        }),
        teams: Vec::new(),
        teams_fail: true,
        groups: AccessGroupMap::new(),
        groups_fail: false,
        calls: Mutex::new(Vec::new()),
    };
    let error = read_caller_rows(&caller(Some("alice"), &[]), &store)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(matches!(error, ScopeError::TeamRead(_)));
}

#[tokio::test]
async fn annotation_group_read_failure() {
    let store = StubReads {
        creators: BTreeMap::new(),
        user: Some(UserScope {
            teams: team_list(&["team-alpha"]),
            models: Vec::new(),
        }),
        teams: vec![restricted("team-alpha", &["gpt-4o"], &["ag-1"])],
        teams_fail: false,
        groups: AccessGroupMap::new(),
        groups_fail: true,
        calls: Mutex::new(Vec::new()),
    };
    let error = read_caller_rows(&caller(Some("alice"), &[]), &store)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(matches!(error, ScopeError::AccessGroupRead(_)));
}

#[test]
fn caller_key_models_come_from_the_auth() {
    let mut fields = BTreeMap::new();
    fields.insert(
        String::from("user_id"),
        JsonData::String(String::from("alice")),
    );
    fields.insert(
        String::from("models"),
        JsonData::Array(vec![JsonData::String(String::from("gpt-4o"))]),
    );
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap();
    let scope = ScopeCaller::from_auth(&auth).unwrap();
    assert_eq!(scope.user_id, Some(String::from("alice")));
    assert_eq!(scope.key_models, vec![String::from("gpt-4o")]);
    assert!(!scope.admin_view);
}

#[tokio::test]
async fn team_models_join_user_deployments() {
    let rows = vec![
        identified("global-gpt", "g1"),
        team_row("team-gpt", "t1", "team-alpha"),
        team_row("other-gpt", "t2", "team-beta"),
    ];
    let store = StubReads {
        creators: BTreeMap::new(),
        user: Some(UserScope {
            teams: team_list(&["team-alpha"]),
            models: Vec::new(),
        }),
        teams: Vec::new(),
        teams_fail: false,
        groups: AccessGroupMap::new(),
        groups_fail: false,
        calls: Mutex::new(Vec::new()),
    };
    let alice = caller(Some("alice"), &[]);
    let stored = read_user_models(rows.clone(), &alice, Some(&store))
        .await
        .unwrap();
    let lookup = snapshot(&rows);
    let kept = non_admin_all_models(stored, &lookup.lookup()).unwrap();
    assert_eq!(ids(&kept), vec![Some("t1")]);
}

// A deployment the user created and also reaches as a team model is listed once, by id.
#[tokio::test]
async fn user_and_team_model_counts_once() {
    let rows = vec![team_row("gpt-4o", "u1", "team-alpha")];
    let mut store = StubReads::default();
    store
        .creators
        .insert("u1".to_owned(), Some("alice".to_owned()));
    store.user = Some(UserScope {
        teams: team_list(&["team-alpha"]),
        models: Vec::new(),
    });
    let alice = caller(Some("alice"), &[]);
    let stored = read_user_models(rows.clone(), &alice, Some(&store))
        .await
        .unwrap();
    let lookup = snapshot(&rows);
    let kept = non_admin_all_models(stored, &lookup.lookup()).unwrap();
    assert_eq!(ids(&kept), vec![Some("u1")]);
}

#[tokio::test]
async fn all_proxy_grant_marks_non_team_models() {
    let rows = vec![
        identified("global-gpt", "g1"),
        team_row("team-gpt", "t1", "team-alpha"),
    ];
    let store = StubReads {
        creators: BTreeMap::new(),
        user: Some(UserScope {
            teams: Vec::new(),
            models: vec![JsonData::String(String::from(ALL_PROXY_MODELS))],
        }),
        teams: Vec::new(),
        teams_fail: false,
        groups: AccessGroupMap::new(),
        groups_fail: false,
        calls: Mutex::new(Vec::new()),
    };
    let alice = caller(Some("alice"), &[]);
    let reading = read_caller_rows(&alice, &store).await.unwrap();
    let lookup = snapshot(&rows);
    let annotated = alice
        .accessible_models(
            rows,
            false,
            reading,
            (&lookup.lookup(), &AccessGroups::default()),
        )
        .unwrap();
    assert_annotations(&annotated[0], true, Some(&[]));
    assert_annotations(&annotated[1], false, None);
}

#[tokio::test]
async fn wildcard_key_keeps_the_user_grant() {
    let rows = vec![
        identified("global-gpt", "g1"),
        team_row("team-gpt", "t1", "team-alpha"),
    ];
    let store = StubReads {
        creators: BTreeMap::new(),
        user: Some(UserScope {
            teams: Vec::new(),
            models: vec![JsonData::String(String::from("team-gpt"))],
        }),
        teams: Vec::new(),
        teams_fail: false,
        groups: AccessGroupMap::new(),
        groups_fail: false,
        calls: Mutex::new(Vec::new()),
    };
    let alice = caller(Some("alice"), &[ALL_PROXY_MODELS, "extra"]);
    let reading = read_caller_rows(&alice, &store).await.unwrap();
    let lookup = snapshot(&rows);
    let annotated = alice
        .accessible_models(
            rows,
            false,
            reading,
            (&lookup.lookup(), &AccessGroups::default()),
        )
        .unwrap();
    assert_annotations(&annotated[0], false, Some(&[]));
    assert_annotations(&annotated[1], true, None);
}

// A key whitelisted to the sentinel alone has no narrowing grant, so the user's team grant stays direct.
#[test]
fn all_proxy_only_key_keeps_the_user_grant() {
    let rows = vec![team_row("team-gpt", "k1", "team-alpha")];
    let snapshot = snapshot(&rows);
    let user_models = vec![String::from("team-gpt")];
    let keys = vec![String::from(ALL_PROXY_MODELS)];
    let direct = direct_access_models(
        &user_models,
        &keys,
        &snapshot.lookup(),
        &AccessGroups::default(),
    )
    .unwrap();
    assert_eq!(direct, vec![String::from("k1")]);
}

#[test]
fn open_team_skips_group_models() {
    let rows = vec![identified("group-model", "m1")];
    let snapshot = snapshot(&rows);
    let mut groups = AccessGroupMap::new();
    groups.insert(String::from("ag-1"), vec![String::from("group-model")]);
    let teams = vec![
        restricted("team-open", &[], &["ag-1"]),
        restricted("team-beta", &["group-model"], &["ag-1"]),
    ];
    let mut team_models = TeamModelMap::new();
    add_access_group_models(&snapshot.lookup(), &teams, &groups, &mut team_models).unwrap();
    let marked: Vec<(&str, Vec<&str>)> = team_models
        .iter()
        .map(|(model_id, team_ids)| {
            (
                model_id.as_str(),
                team_ids.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(marked, vec![("m1", vec!["team-beta"])]);
}
