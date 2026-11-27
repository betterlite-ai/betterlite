use async_trait::async_trait;
use betterlite_auth::auth_key_lookup::error::KeyLookupError;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use std::sync::Mutex;

use super::super::enrich::{
    FailureIdentityLookup, IdentityLookupError, KeyFacts, enrich_failure_metadata,
};
use super::support::object;

#[derive(Default)]
struct Lookup {
    key: Mutex<Option<Result<KeyFacts, KeyLookupError>>>,
    team: Mutex<Option<Result<Option<String>, TeamLookupError>>>,
    key_calls: Mutex<Vec<String>>,
    team_calls: Mutex<Vec<String>>,
}

impl Lookup {
    fn with_key(facts: KeyFacts) -> Self {
        Self {
            key: Mutex::new(Some(Ok(facts))),
            // ast-grep-ignore: no-default-struct-update - the fixture sets only the fields it reads
            ..Self::default()
        }
    }

    fn failing_key() -> Self {
        Self {
            key: Mutex::new(Some(Err(KeyLookupError::NoDatabase))),
            // ast-grep-ignore: no-default-struct-update - the fixture sets only the fields it reads
            ..Self::default()
        }
    }

    fn with_team(alias: Option<&str>) -> Self {
        Self {
            team: Mutex::new(Some(Ok(alias.map(str::to_owned)))),
            // ast-grep-ignore: no-default-struct-update - the fixture sets only the fields it reads
            ..Self::default()
        }
    }

    fn failing_team() -> Self {
        Self {
            team: Mutex::new(Some(Err(TeamLookupError::NoDatabase))),
            // ast-grep-ignore: no-default-struct-update - the fixture sets only the fields it reads
            ..Self::default()
        }
    }

    fn key_calls(&self) -> Vec<String> {
        self.key_calls.lock().expect("the key log").clone()
    }

    fn team_calls(&self) -> Vec<String> {
        self.team_calls.lock().expect("the team log").clone()
    }
}

#[async_trait]
impl FailureIdentityLookup for Lookup {
    async fn key_facts(&self, hashed_token: &str) -> Result<KeyFacts, IdentityLookupError> {
        self.key_calls
            .lock()
            .expect("the key log")
            .push(hashed_token.to_owned());
        let scripted = self.key.lock().expect("the key script").take();
        scripted
            .expect("the key script is spent once")
            .map_err(IdentityLookupError::from)
    }

    async fn team_alias(&self, team_id: &str) -> Result<Option<String>, IdentityLookupError> {
        self.team_calls
            .lock()
            .expect("the team log")
            .push(team_id.to_owned());
        let scripted = self.team.lock().expect("the team script").take();
        scripted
            .expect("the team script is spent once")
            .map_err(IdentityLookupError::from)
    }
}

fn null_metadata() -> JsonObject {
    object(
        r#"{"user_api_key": "hash",
            "user_api_key_alias": null,
            "user_api_key_team_id": null,
            "user_api_key_team_alias": null}"#,
    )
}

async fn assert_falsey_hash_skips(hash: Json) {
    let lookup = Lookup::default();
    let mut metadata = object(r#"{"user_api_key_team_id": "team"}"#);
    metadata.insert("user_api_key".to_owned(), hash);
    enrich_failure_metadata(&mut metadata, Some(&lookup), true).await;
    assert!(lookup.key_calls().is_empty());
    assert!(lookup.team_calls().is_empty());
}

#[tokio::test]
async fn a_falsey_key_hash_skips_every_lookup() {
    assert_falsey_hash_skips(Json::Null).await;
    assert_falsey_hash_skips(Json::Text(String::new())).await;
    assert_falsey_hash_skips(Json::Bool(false)).await;
    assert_falsey_hash_skips(Json::Number(serde_json::Number::from(0_u8))).await;
}

#[tokio::test]
async fn a_null_alias_fills_every_null_identity_field() {
    let lookup = Lookup::with_key(KeyFacts {
        key_alias: Some("alias".to_owned()),
        user_id: Some("user".to_owned()),
        team_id: Some("team".to_owned()),
        org_id: None,
        project_id: Some("project".to_owned()),
    });
    let mut metadata = null_metadata();
    enrich_failure_metadata(&mut metadata, Some(&lookup), true).await;
    assert_eq!(lookup.key_calls(), vec!["hash".to_owned()]);
    assert_eq!(
        metadata["user_api_key_alias"],
        Json::Text("alias".to_owned())
    );
    assert_eq!(
        metadata["user_api_key_user_id"],
        Json::Text("user".to_owned())
    );
    assert_eq!(
        metadata["user_api_key_team_id"],
        Json::Text("team".to_owned())
    );
    assert_eq!(
        metadata["user_api_key_project_id"],
        Json::Text("project".to_owned())
    );
    assert_eq!(metadata["user_api_key_org_id"], Json::Null);
}

#[tokio::test]
async fn a_missing_alias_key_is_inserted_as_null() {
    let lookup = Lookup::with_key(KeyFacts::default());
    let mut metadata = object(r#"{"user_api_key": "hash"}"#);
    enrich_failure_metadata(&mut metadata, Some(&lookup), true).await;
    assert_eq!(metadata["user_api_key_alias"], Json::Null);
    assert_eq!(metadata["user_api_key_user_id"], Json::Null);
}

#[tokio::test]
async fn a_present_identity_is_never_overwritten() {
    let lookup = Lookup::with_key(KeyFacts {
        key_alias: Some("alias".to_owned()),
        user_id: Some("new-user".to_owned()),
        // ast-grep-ignore: no-default-struct-update - the fixture sets only the fields it reads
        ..KeyFacts::default()
    });
    let mut metadata = object(
        r#"{"user_api_key": "hash", "user_api_key_alias": null,
            "user_api_key_user_id": "kept"}"#,
    );
    enrich_failure_metadata(&mut metadata, Some(&lookup), true).await;
    assert_eq!(
        metadata["user_api_key_alias"],
        Json::Text("alias".to_owned())
    );
    assert_eq!(
        metadata["user_api_key_user_id"],
        Json::Text("kept".to_owned())
    );
}

#[tokio::test]
async fn an_empty_alias_blocks_the_key_lookup_but_not_the_team_lookup() {
    let lookup = Lookup::with_team(Some("team-alias"));
    let mut metadata = object(
        r#"{"user_api_key": "hash", "user_api_key_alias": "",
            "user_api_key_team_id": "team", "user_api_key_team_alias": null}"#,
    );
    enrich_failure_metadata(&mut metadata, Some(&lookup), true).await;
    assert!(lookup.key_calls().is_empty());
    assert_eq!(lookup.team_calls(), vec!["team".to_owned()]);
    assert_eq!(
        metadata["user_api_key_team_alias"],
        Json::Text("team-alias".to_owned())
    );
}

#[tokio::test]
async fn a_delayed_event_keeps_its_identity_but_fills_the_team_alias() {
    let key = Lookup::with_key(KeyFacts {
        user_id: Some("new-user".to_owned()),
        // ast-grep-ignore: no-default-struct-update - the fixture sets only the fields it reads
        ..KeyFacts::default()
    });
    let mut metadata = object(
        r#"{"user_api_key": "hash", "user_api_key_alias": null,
            "user_api_key_team_id": "team", "user_api_key_team_alias": null}"#,
    );
    enrich_failure_metadata(&mut metadata, Some(&key), false).await;
    assert!(key.key_calls().is_empty());
    let team = Lookup::with_team(Some("team-alias"));
    enrich_failure_metadata(&mut metadata, Some(&team), false).await;
    assert_eq!(lookup_alias(&metadata), Some("team-alias"));
    assert_eq!(metadata["user_api_key_alias"], Json::Null);
}

fn lookup_alias(metadata: &JsonObject) -> Option<&str> {
    match metadata.get("user_api_key_team_alias") {
        Some(Json::Text(alias)) => Some(alias.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

#[tokio::test]
async fn a_known_team_alias_is_not_looked_up_again() {
    let lookup = Lookup::with_team(Some("other"));
    let mut metadata = object(
        r#"{"user_api_key": "hash", "user_api_key_alias": "alias",
            "user_api_key_team_id": "team", "user_api_key_team_alias": "known"}"#,
    );
    enrich_failure_metadata(&mut metadata, Some(&lookup), true).await;
    assert!(lookup.team_calls().is_empty());
    assert_eq!(
        metadata["user_api_key_team_alias"],
        Json::Text("known".to_owned())
    );
}

#[tokio::test]
async fn a_team_without_an_alias_inserts_nothing() {
    let lookup = Lookup::with_team(None);
    let mut metadata = null_metadata();
    metadata.insert(
        "user_api_key_team_id".to_owned(),
        Json::Text("team".to_owned()),
    );
    enrich_failure_metadata(&mut metadata, Some(&lookup), true).await;
    assert_eq!(lookup.team_calls(), vec!["team".to_owned()]);
    assert_eq!(metadata["user_api_key_team_alias"], Json::Null);
}

#[tokio::test]
async fn lookup_errors_are_debug_only_and_the_team_step_still_runs() {
    let key = Lookup::failing_key();
    let mut metadata = null_metadata();
    metadata.insert(
        "user_api_key_team_id".to_owned(),
        Json::Text("team".to_owned()),
    );
    enrich_failure_metadata(&mut metadata, Some(&key), true).await;
    assert_eq!(metadata["user_api_key_alias"], Json::Null);
    let team = Lookup::failing_team();
    enrich_failure_metadata(&mut metadata, Some(&team), true).await;
    assert_eq!(metadata["user_api_key_team_alias"], Json::Null);
    assert_eq!(team.team_calls(), vec!["team".to_owned()]);
}

#[tokio::test]
async fn a_missing_team_id_skips_the_team_lookup() {
    let lookup = Lookup::with_team(Some("alias"));
    let mut metadata = object(
        r#"{"user_api_key": "hash", "user_api_key_alias": "alias",
            "user_api_key_team_id": "", "user_api_key_team_alias": null}"#,
    );
    enrich_failure_metadata(&mut metadata, Some(&lookup), true).await;
    assert!(lookup.team_calls().is_empty());
}

#[tokio::test]
async fn no_lookup_port_leaves_the_metadata_untouched() {
    let mut metadata = null_metadata();
    let before = metadata.clone();
    enrich_failure_metadata(&mut metadata, None, true).await;
    assert_eq!(metadata, before);
}
