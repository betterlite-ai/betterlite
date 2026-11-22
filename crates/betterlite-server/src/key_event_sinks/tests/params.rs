use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use secrecy::SecretString;
use tracing::Level;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::support::{
    CachedTeams, Call, FakeTeams, Logs, manager, object, settings, team, team_lookup,
};
use crate::key_event_sinks::error::SecretSinkError;
use crate::key_event_sinks::secrets::SecretSink;
use crate::key_event_sinks::team_params::{TeamSecretSettings, secret_manager_settings};

fn teams_with(metadata: &str) -> FakeTeams {
    FakeTeams {
        teams: BTreeMap::from([("t1".to_owned(), team("t1", metadata))]),
        lookups: Mutex::default(),
    }
}

#[tokio::test]
async fn params_missing_runtime_db_cache() {
    let fake = Arc::new(CachedTeams {
        team: Some(team(
            "t1",
            r#"{"secret_manager_settings": {"region": "region-eu"}}"#,
        )),
        reads: Mutex::default(),
    });
    assert_eq!(secret_manager_settings(None, Some("t1")).await, None);
    let without_database = team_lookup(&fake, false);
    assert_eq!(
        secret_manager_settings(Some(&without_database), Some("t1")).await,
        None
    );
    assert!(fake.reads.lock().unwrap().is_empty());
    let ready = team_lookup(&fake, true);
    assert_eq!(secret_manager_settings(Some(&ready), None).await, None);
    assert!(fake.reads.lock().unwrap().is_empty());
    let found = secret_manager_settings(Some(&ready), Some("t1")).await;
    assert_eq!(found, Some(object(r#"{"region": "region-eu"}"#)));
    assert_eq!(*fake.reads.lock().unwrap(), ["team_id:t1"]);
}

#[tokio::test]
async fn params_lookup_error_debug_none() {
    let teams = FakeTeams::default();
    let logs = Logs::default();
    let _logging = set_default(tracing_subscriber::registry().with(logs.clone()));
    assert_eq!(
        secret_manager_settings(Some(&teams), Some("t9")).await,
        None
    );
    assert_eq!(*teams.lookups.lock().unwrap(), ["t9"]);
    let lines = logs.lines();
    assert_eq!(lines.len(), 1);
    assert!(
        lines
            .iter()
            .all(|(level, message)| *level == Level::DEBUG && message.contains("team_id=t9"))
    );
}

#[rstest]
#[case::settings(
    r#"{"secret_manager_settings": {"region": "region-eu", "nested": {"a": 1}}}"#,
    true
)]
#[case::null_metadata("null", false)]
#[case::no_settings(r#"{"other": {"region": "region-eu"}}"#, false)]
#[case::null_settings(r#"{"secret_manager_settings": null}"#, false)]
#[case::empty_settings(r#"{"secret_manager_settings": {}}"#, false)]
#[case::text_settings(r#"{"secret_manager_settings": "region-eu"}"#, false)]
#[case::list_settings(r#"{"secret_manager_settings": [{"region": "region-eu"}]}"#, false)]
#[tokio::test]
async fn params_model_dump_and_nonempty_dict(#[case] metadata: &str, #[case] found: bool) {
    let teams = teams_with(metadata);
    let settings = secret_manager_settings(Some(&teams), Some("t1")).await;
    let expected = object(r#"{"region": "region-eu", "nested": {"a": 1}}"#);
    assert_eq!(settings, found.then_some(expected));
}

// a manager whose own params need a text region, as a typed manager reads team settings
#[derive(Clone, Debug, PartialEq)]
struct Region(String);

#[derive(Debug, thiserror::Error)]
#[error("the region is not text")]
struct NotText;

impl TryFrom<TeamSecretSettings> for Region {
    type Error = NotText;

    fn try_from(settings: TeamSecretSettings) -> Result<Self, NotText> {
        match settings.get("region") {
            Some(JsonData::String(region)) => Ok(Self(region.clone())),
            Some(_) | None => Err(NotText),
        }
    }
}

#[tokio::test]
async fn params_model_dump_error_propagates() {
    let config = settings(true, "litellm/");
    let fake = manager::<Region>(&[], &[]);
    let teams = teams_with(r#"{"secret_manager_settings": {"region": 5}}"#);
    let sink = SecretSink {
        settings: &config,
        manager: Some(&fake),
        teams: Some(&teams),
    };
    let error = sink
        .store("alias", &SecretString::from("sk-a"), Some("t1"))
        .await
        .unwrap_err();
    assert!(matches!(error, SecretSinkError::Params { source: NotText }));
    assert!(fake.calls().is_empty());
    let typed = teams_with(r#"{"secret_manager_settings": {"region": "region-eu"}}"#);
    let typed_sink = SecretSink {
        settings: &config,
        manager: Some(&fake),
        teams: Some(&typed),
    };
    typed_sink.rename("old", "new", Some("t1")).await.unwrap();
    let read = Call::Read {
        name: "litellm/old".to_owned(),
        params: Some(Region("region-eu".to_owned())),
    };
    assert_eq!(fake.calls(), [read]);
}
