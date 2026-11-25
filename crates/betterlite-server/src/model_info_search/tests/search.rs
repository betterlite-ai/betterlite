use std::collections::BTreeSet;
use std::convert::Infallible;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::support::{caller, decryption, model, named, names, stored_row, text};
use crate::model_info_search::search::{
    ModelFilter, SearchDatabase, SearchDatabaseError, SearchOptions, SearchSources,
    apply_search_filter,
};
use crate::model_team_scope::lookup::Decryption;
use crate::model_team_scope::reads::{TeamScopeReadError, TeamScopeReads};
use crate::team_access::caller::TeamCaller;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Count(ModelFilter),
    Find(ModelFilter, i64),
}

struct FakeDatabase {
    rows: Vec<ProxyModelTable>,
    calls: Mutex<Vec<Call>>,
    fail_count: bool,
    fail_find: bool,
}

impl FakeDatabase {
    fn new(rows: Vec<ProxyModelTable>, fail_count: bool, fail_find: bool) -> Self {
        Self {
            rows,
            calls: Mutex::new(Vec::new()),
            fail_count,
            fail_find,
        }
    }

    fn record(&self, call: Call) {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(call);
    }

    fn calls(&self) -> Vec<Call> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }

    fn matching(&self, filter: &ModelFilter) -> Vec<ProxyModelTable> {
        self.rows
            .iter()
            .filter(|row| name_matches(&row.fields.model_name, filter))
            .filter(|row| !filter.excluded_ids.contains(&row.fields.model_id))
            .cloned()
            .collect()
    }
}

fn name_matches(name: &str, filter: &ModelFilter) -> bool {
    let Some(exact) = &filter.exact_name else {
        return name.to_lowercase().contains(&filter.search_lower);
    };
    name == exact
}

fn database_failure() -> SearchDatabaseError {
    let missing = DbError::<Infallible>::NoDbConnected;
    SearchDatabaseError::Database(TableError::from(missing))
}

#[async_trait]
impl SearchDatabase for FakeDatabase {
    async fn count(&self, filter: &ModelFilter) -> Result<i64, SearchDatabaseError> {
        self.record(Call::Count(filter.clone()));
        if self.fail_count {
            return Err(database_failure());
        }
        Ok(i64::try_from(self.matching(filter).len()).unwrap_or(i64::MAX))
    }

    async fn find(
        &self,
        filter: &ModelFilter,
        take: i64,
    ) -> Result<Vec<ProxyModelTable>, SearchDatabaseError> {
        self.record(Call::Find(filter.clone(), take));
        if self.fail_find {
            return Err(database_failure());
        }
        let limit = usize::try_from(take).unwrap_or(0);
        Ok(self.matching(filter).into_iter().take(limit).collect())
    }
}

struct FakeReads {
    teams: Vec<String>,
}

#[async_trait]
impl TeamScopeReads for FakeReads {
    async fn user_teams(&self, _user_id: &str) -> Result<Option<Vec<String>>, TeamScopeReadError> {
        Ok(Some(self.teams.clone()))
    }

    async fn team_models(&self, _team_id: &str) -> Result<Option<Vec<String>>, TeamScopeReadError> {
        Ok(None)
    }

    async fn model_ids_by_names(
        &self,
        _names: &[String],
    ) -> Result<Vec<String>, TeamScopeReadError> {
        Ok(Vec::new())
    }

    async fn model_row(
        &self,
        _model_id: &str,
    ) -> Result<Option<ProxyModelTable>, TeamScopeReadError> {
        Ok(None)
    }
}

fn searchable_row(model_id: &str, model_name: &str) -> ProxyModelTable {
    stored_row(model_id, model_name, r#"{"model":"openai/gpt-4o"}"#, "{}")
}

fn team_model(name: &str, team_id: &str) -> JsonObject {
    model(
        name,
        vec![
            ("db_model", Json::Bool(true)),
            ("id", text(name)),
            ("team_id", text(team_id)),
        ],
    )
}

fn expected_filter(search_lower: &str, exact_name: Option<&str>) -> ModelFilter {
    ModelFilter {
        search_lower: search_lower.to_owned(),
        exact_name: exact_name.map(str::to_owned),
        excluded_ids: BTreeSet::new(),
    }
}

fn unowned_sources<'a>(
    db: &'a FakeDatabase,
    decrypt: &'a Decryption,
    reads: Option<&'a FakeReads>,
    who: Option<&'a TeamCaller>,
) -> SearchSources<'a, FakeDatabase, FakeReads> {
    SearchSources {
        database: db,
        decrypt,
        reads,
        caller: who,
    }
}

#[tokio::test]
async fn model_name_search() {
    let db = FakeDatabase::new(Vec::new(), false, false);
    let decrypt = decryption();
    let models = vec![
        named("GPT-4O"),
        named("gpt-4o-mini"),
        model(
            "internal-byok-name",
            vec![("team_public_model_name", text("Claude-3"))],
        ),
        named("other-model"),
    ];
    let options = SearchOptions::default();
    let no_sources: Option<&SearchSources<'_, FakeDatabase, FakeReads>> = None;
    let matched = apply_search_filter(&models, "gpt-4o", no_sources, &options).await;
    assert_eq!(names(&matched.models), ["GPT-4O", "gpt-4o-mini"]);
    assert_eq!(matched.total_count, Some(2));
    let public_search = apply_search_filter(&models, "  claude ", no_sources, &options).await;
    assert_eq!(names(&public_search.models), ["internal-byok-name"]);
    assert_eq!(public_search.total_count, Some(1));
    let missing = apply_search_filter(&models, "nomatch", no_sources, &options).await;
    assert!(missing.models.is_empty());
    assert_eq!(missing.total_count, Some(0));
    let blank = apply_search_filter(&models, "   ", no_sources, &options).await;
    assert_eq!(blank.models, models);
    assert_eq!(blank.total_count, None);
    let empty = apply_search_filter(&models, "", no_sources, &options).await;
    assert_eq!(empty.models, models);
    assert_eq!(empty.total_count, None);
    let sources = unowned_sources(&db, &decrypt, None, None);
    let blank_with_db = apply_search_filter(&models, "  ", Some(&sources), &options).await;
    assert_eq!(blank_with_db.models, models);
    assert_eq!(blank_with_db.total_count, None);
    assert!(db.calls().is_empty());
}

#[tokio::test]
async fn byok_scope_retained() {
    let db = FakeDatabase::new(Vec::new(), false, false);
    let decrypt = decryption();
    let reads = FakeReads {
        teams: vec![String::from("team-b")],
    };
    let who = caller("alice", "team-a");
    let models = vec![
        team_model("team-a-model", "team-a"),
        team_model("team-b-model", "team-b"),
        team_model("team-c-model", "team-c"),
        named("plain-model"),
    ];
    let sources = unowned_sources(&db, &decrypt, Some(&reads), Some(&who));
    let options = SearchOptions::default();
    let outcome = apply_search_filter(&models, "model", Some(&sources), &options).await;
    assert_eq!(
        names(&outcome.models),
        ["team-a-model", "team-b-model", "plain-model"]
    );
    assert_eq!(outcome.total_count, Some(3));
    let foreign = apply_search_filter(&models, "team-c", Some(&sources), &options).await;
    assert!(foreign.models.is_empty());
    assert_eq!(foreign.total_count, Some(0));
}

#[tokio::test]
async fn a_count_error_keeps_router_models() {
    let db = FakeDatabase::new(vec![searchable_row("d1", "gpt-4o-db")], true, false);
    let decrypt = decryption();
    let sources = unowned_sources(&db, &decrypt, None, None);
    let models = vec![named("gpt-4o")];
    let options = SearchOptions::default();
    let outcome = apply_search_filter(&models, "gpt", Some(&sources), &options).await;
    assert_eq!(names(&outcome.models), ["gpt-4o"]);
    assert_eq!(outcome.total_count, Some(1));
    assert_eq!(db.calls(), [Call::Count(expected_filter("gpt", None))]);
}

#[tokio::test]
async fn a_fetch_error_keeps_router_models() {
    let db = FakeDatabase::new(vec![searchable_row("d1", "gpt-4o-db")], false, true);
    let decrypt = decryption();
    let sources = unowned_sources(&db, &decrypt, None, None);
    let models = vec![named("gpt-4o")];
    let options = SearchOptions::default();
    let outcome = apply_search_filter(&models, "gpt", Some(&sources), &options).await;
    assert_eq!(names(&outcome.models), ["gpt-4o"]);
    assert_eq!(outcome.total_count, Some(1));
    assert_eq!(
        db.calls(),
        [
            Call::Count(expected_filter("gpt", None)),
            Call::Find(expected_filter("gpt", None), 49),
        ]
    );
}

// A decrypt failure inside one row drops the whole DB batch and keeps the router-side matches.
#[tokio::test]
async fn a_decrypt_error_keeps_router_models() {
    let db = FakeDatabase::new(
        vec![
            searchable_row("d1", "gpt-4o-db"),
            stored_row(
                "d2",
                "gpt-4o-broken",
                r#"{"model":"openai/gpt-4o","api_key":"os.environ/oidc/nope/aud"}"#,
                "{}",
            ),
        ],
        false,
        false,
    );
    let decrypt = decryption();
    let sources = unowned_sources(&db, &decrypt, None, None);
    let models = vec![named("gpt-4o")];
    let options = SearchOptions::default();
    let outcome = apply_search_filter(&models, "gpt", Some(&sources), &options).await;
    assert_eq!(names(&outcome.models), ["gpt-4o"]);
    assert_eq!(outcome.total_count, Some(1));
}
