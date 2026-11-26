use std::fs::read;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::auth_context_sources::router_settings::{
    MAX_ROUTER_SETTINGS_BYTES, TeamRouterSettings as _, decode_router_settings,
    hierarchical_router_settings, parse_router_settings_value,
};
use crate::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use crate::auth_prefetch::permission::store::ObjectPermissionStore;
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use crate::auth_team_lookup::ports::{StoreAnswer, TeamCache, TeamCacheError, TeamStore};
use crate::auth_user_lookup::cadence::{Cadence, WallClock};

use super::support::{FakeTeams, TeamAnswer, json, object, team_columns};

const TEAM_ALIASES: &str = r#"{"model_group_alias": {"team-alias": "team-target"}}"#;
const KEY_ALIASES: &str = r#"{"model_group_alias": {"key-alias": "key-target"}}"#;

#[tokio::test]
async fn settings_key_before_team() {
    let key = json(KEY_ALIASES);
    let teams = FakeTeams::new(TeamAnswer::Settings(team_columns(TEAM_ALIASES)));
    let resolved = hierarchical_router_settings(Some(&key), Some("team-one"), &teams)
        .await
        .expect("the key settings resolve");
    assert_eq!(resolved, object(KEY_ALIASES));
    assert!(teams.calls().is_empty());
}

#[rstest]
#[case::null(json("null"))]
#[case::empty_object(json("{}"))]
#[case::empty_string(json(r#""""#))]
#[case::empty_list(json("[]"))]
#[case::number(json("3"))]
#[case::empty_object_text(json(r#""{}""#))]
#[case::invalid_yaml(json(r#""not: [valid""#))]
#[tokio::test]
async fn empty_key_falls_back(#[case] empty: Json) {
    let teams = FakeTeams::new(TeamAnswer::Settings(team_columns(TEAM_ALIASES)));
    let resolved = hierarchical_router_settings(Some(&empty), Some("team-one"), &teams).await;
    assert_eq!(resolved, Some(object(TEAM_ALIASES)), "{empty:?}");
    assert_eq!(teams.calls(), vec!["team-one".to_owned()]);
}

#[tokio::test]
async fn missing_key_falls_back() {
    let teams = FakeTeams::new(TeamAnswer::Settings(team_columns(TEAM_ALIASES)));
    let resolved = hierarchical_router_settings(None, Some("team-one"), &teams).await;
    assert_eq!(resolved, Some(object(TEAM_ALIASES)));
    assert_eq!(teams.calls(), vec!["team-one".to_owned()]);
}

#[tokio::test]
async fn global_settings_not_merged() {
    // the key's settings answer alone; the team row is never read and never merged in
    let key = json(KEY_ALIASES);
    let teams = FakeTeams::new(TeamAnswer::Settings(team_columns(r#"{"num_retries": 3}"#)));
    let resolved = hierarchical_router_settings(Some(&key), Some("team-one"), &teams)
        .await
        .expect("the key settings resolve");
    assert_eq!(resolved, object(KEY_ALIASES));
    assert!(teams.calls().is_empty());

    // with no key and no team there is no third, global source to consult
    let global_only = FakeTeams::new(TeamAnswer::Settings(team_columns(TEAM_ALIASES)));
    assert_eq!(
        hierarchical_router_settings(None, None, &global_only).await,
        None
    );
    assert!(global_only.calls().is_empty());
}

#[tokio::test]
async fn team_error_silent_drop() {
    let failing = FakeTeams::new(TeamAnswer::Failure);
    assert_eq!(
        hierarchical_router_settings(None, Some("team-one"), &failing).await,
        None
    );
    assert_eq!(failing.calls(), vec!["team-one".to_owned()]);

    let absent = FakeTeams::new(TeamAnswer::Absent);
    assert_eq!(
        hierarchical_router_settings(None, Some("team-one"), &absent).await,
        None
    );
    assert_eq!(absent.calls(), vec!["team-one".to_owned()]);
}

#[test]
fn settings_yaml_json_nonempty_object() {
    assert_eq!(parse_router_settings_value(None), None);
    assert_eq!(parse_router_settings_value(Some(&Json::Null)), None);
    assert_eq!(
        parse_router_settings_value(Some(&Json::Object(JsonObject::new()))),
        None
    );
    let entry = json(KEY_ALIASES);
    assert_eq!(
        parse_router_settings_value(Some(&entry)),
        Some(object(KEY_ALIASES))
    );
    let yaml = json(r#""model_group_alias:\n  key-alias: key-target""#);
    assert_eq!(
        parse_router_settings_value(Some(&yaml)),
        Some(object(KEY_ALIASES))
    );
    let json_text = json(r#""{\"model_group_alias\": {\"key-alias\": \"key-target\"}}""#);
    assert_eq!(
        parse_router_settings_value(Some(&json_text)),
        Some(object(KEY_ALIASES))
    );
}

#[rstest]
#[case::true_value(json("true"))]
#[case::number(json("3"))]
#[case::list(json("[1]"))]
#[case::empty_list_text(json(r#""[]""#))]
#[case::number_text(json(r#""3""#))]
#[case::empty_text(json(r#""""#))]
#[case::empty_object_text(json(r#""{}""#))]
#[case::invalid_yaml(json(r#""not: [valid""#))]
fn unusable_settings_value_is_none(#[case] value: Json) {
    assert_eq!(parse_router_settings_value(Some(&value)), None, "{value:?}");
}

fn nested(lists: usize, leaf: &str) -> String {
    let mut text = String::from(r#"{"model_group_alias":"#);
    text.push_str(&"[".repeat(lists));
    text.push_str(leaf);
    text.push_str(&"]".repeat(lists));
    text.push('}');
    text
}

#[test]
fn settings_size_depth_limits() {
    // a settings string exactly at the byte cap still decodes
    let mut text = String::from("model_group_alias: {}\n");
    text.push_str(&"#".repeat(MAX_ROUTER_SETTINGS_BYTES.saturating_sub(text.len())));
    assert_eq!(text.len(), MAX_ROUTER_SETTINGS_BYTES);
    assert_eq!(
        decode_router_settings(text.as_bytes()),
        Some(object(r#"{"model_group_alias": {}}"#))
    );
    text.push('#');
    assert_eq!(decode_router_settings(text.as_bytes()), None);

    // 64 containers decode and 65 do not, on the YAML and the JSON boundary alike
    assert!(decode_router_settings(nested(63, "1").as_bytes()).is_some());
    assert!(decode_router_settings(nested(64, "1").as_bytes()).is_none());

    // invalid UTF-8 never reaches a decoder as text
    assert_eq!(decode_router_settings(&[0xff_u8, 0xfe]), None);

    // duplicate JSON keys fail the YAML boundary and decode through the JSON fallback
    assert_eq!(
        decode_router_settings(
            br#"{"model_group_alias": {"key-alias": "first"}, "model_group_alias": {"key-alias": "last"}}"#
        ),
        Some(object(r#"{"model_group_alias": {"key-alias": "last"}}"#))
    );
}

#[derive(Debug)]
struct ScriptedCache(Option<LiteLLMTeamTableCachedObj>);

#[async_trait]
impl TeamCache for ScriptedCache {
    async fn read_team(
        &self,
        _key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        Ok(self.0.clone())
    }

    async fn write_team(
        &self,
        _key: &str,
        _team: &LiteLLMTeamTableCachedObj,
        _ttl: f64,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn cache_team(
        &self,
        _team_id: &str,
        _team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn read_access_group(
        &self,
        _key: &str,
    ) -> Result<Option<AccessGroupTable>, TeamCacheError> {
        Ok(None)
    }

    async fn cache_access_group(
        &self,
        _access_group_id: &str,
        _table: &AccessGroupTable,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }
}

#[derive(Debug)]
struct NoPermissionCache;

#[async_trait]
impl ObjectPermissionCache for NoPermissionCache {
    async fn read(&self, _key: &str) -> PermissionAnswer {
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _permission: &LiteLLMObjectPermissionTable,
        _ttl: f64,
    ) -> Result<(), PermissionCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        0.0_f64
    }
}

#[derive(Debug)]
struct NoTeamStore;

#[async_trait]
impl TeamStore for NoTeamStore {
    async fn find_team(&self, _team_id: &str, _use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        Ok(None)
    }

    async fn find_by_alias(&self, _team_alias: &str) -> StoreAnswer<Vec<JsonData>> {
        Ok(Vec::new())
    }

    async fn find_access_group(
        &self,
        _access_group_id: &str,
        _use_writer: bool,
    ) -> StoreAnswer<Option<JsonData>> {
        Ok(None)
    }
}

#[derive(Debug)]
struct NoPermissionStore;

#[async_trait]
impl ObjectPermissionStore for NoPermissionStore {
    async fn find_permission(&self, _id: &str) -> Result<Option<JsonData>, PermissionError> {
        Ok(None)
    }
}

fn lookup_with(cached: Option<LiteLLMTeamTableCachedObj>) -> TeamLookup {
    TeamLookup {
        cache: Arc::new(ScriptedCache(cached)),
        permission_cache: Arc::new(NoPermissionCache),
        database: Some(TeamDatabase {
            store: Arc::new(NoTeamStore),
            permission_reader: Arc::new(NoPermissionStore),
            permission_writer: Arc::new(NoPermissionStore),
        }),
        provisioner: None,
        cadence: Arc::new(Cadence::new(4_usize, Arc::new(WallClock))),
        in_memory_ttl: 60_i32,
    }
}

fn cached_team_row() -> LiteLLMTeamTableCachedObj {
    let row = team_columns(
        r#"{"team_id": "team-one", "team_alias": "cached", "models": ["gpt-4o"], "object_permission_id": null, "router_settings": {"model_group_alias": {"key-alias": "key-target"}}}"#,
    );
    LiteLLMTeamTableCachedObj::try_from(JsonData::Object(row)).expect("the team row decodes")
}

#[tokio::test]
async fn team_settings_through_b737() {
    let lookup = lookup_with(Some(cached_team_row()));
    let resolved = hierarchical_router_settings(None, Some("team-one"), &lookup)
        .await
        .expect("the cached team settings resolve");
    assert_eq!(resolved, object(KEY_ALIASES));

    // a miss reads through the database, and an absent row is the team-not-found answer
    let missing = lookup_with(None);
    let error = missing
        .router_settings("team-one")
        .await
        .expect_err("a missing team row is an error");
    assert!(matches!(error, TeamLookupError::TeamNotFound { .. }));
}

#[rstest]
fn replays_the_auth_context_sources_corpus(
    #[files("fuzz/corpus/auth_context_sources/*")] path: PathBuf,
) {
    let bytes = read(&path).expect("the corpus file is readable");
    let decoded = decode_router_settings(&bytes);
    assert!(
        decoded.is_none_or(|settings| !settings.is_empty()),
        "{path:?}"
    );
}

#[tokio::test]
async fn team_settings_keep_all_json_types() {
    let document = r#"{"null":null,"bool":true,"number":42,"array":[false,7,"kept"],"object":{"value":"text"}}"#;
    let teams = FakeTeams::new(TeamAnswer::Settings(team_columns(document)));
    assert_eq!(
        hierarchical_router_settings(None, Some("team-one"), &teams).await,
        Some(object(document))
    );
}

#[rstest]
#[case::empty_array("[]", false)]
#[case::empty_object("{}", false)]
#[case::boolean("false", true)]
#[case::null("null", true)]
#[case::number("0", true)]
#[case::string(r#""text""#, true)]
fn settings_leaf_depth_boundary(#[case] leaf: &str, #[case] fits_at_limit: bool) {
    assert!(decode_router_settings(nested(62, leaf).as_bytes()).is_some());
    assert_eq!(
        decode_router_settings(nested(63, leaf).as_bytes()).is_some(),
        fits_at_limit
    );
    assert!(decode_router_settings(nested(64, leaf).as_bytes()).is_none());
}
