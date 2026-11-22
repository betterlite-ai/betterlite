use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_config::sections::key_event_sinks::Settings;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_secret_managers::secret_contracts::options::SecretTags;
use rstest::rstest;
use secrecy::SecretString;
use tracing::Level;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::support::{Call, FakeManager, FakeTeams, Logs, manager, object, settings, team};
use crate::key_event_sinks::secrets::SecretSink;
use crate::key_event_sinks::team_params::TeamSecretSettings as Params;

fn sink<'deps>(
    settings: &'deps Settings,
    manager: Option<&'deps FakeManager<Params>>,
    teams: &'deps FakeTeams,
) -> SecretSink<'deps, FakeManager<Params>> {
    SecretSink {
        settings,
        manager,
        teams: Some(teams),
    }
}

fn regional_teams() -> FakeTeams {
    let teams = [
        (
            "t1",
            r#"{"secret_manager_settings": {"region": "region-eu"}}"#,
        ),
        (
            "t2",
            r#"{"secret_manager_settings": {"region": "region-us"}}"#,
        ),
    ];
    FakeTeams {
        teams: teams
            .into_iter()
            .map(|(id, metadata)| (id.to_owned(), team(id, metadata)))
            .collect(),
        lookups: Mutex::default(),
    }
}

fn key(token: &str, key_alias: Option<&str>, team_id: Option<&str>) -> LiteLLMVerificationToken {
    let alias = serde_json::to_string(&key_alias).unwrap();
    let team = serde_json::to_string(&team_id).unwrap();
    let row = format!(r#"{{"token": "{token}", "key_alias": {alias}, "team_id": {team}}}"#);
    serde_json::from_str(&row).unwrap()
}

#[rstest]
#[case::switch_off(false, true, 0)]
#[case::no_client(true, false, 0)]
#[case::both(true, true, 5)]
#[tokio::test]
async fn secret_gate_settings_switch_client(
    #[case] store_virtual_keys: bool,
    #[case] client: bool,
    #[case] calls: usize,
) {
    let config = settings(store_virtual_keys, "litellm/");
    let fake = manager(&[("litellm/old", "sk-old")], &[]);
    let teams = regional_teams();
    let secrets = sink(&config, client.then_some(&fake), &teams);
    let value = SecretString::from("sk-new");
    secrets.store("name", &value, Some("t1")).await.unwrap();
    secrets
        .rotate("old", "new", &value, Some("t1"))
        .await
        .unwrap();
    secrets.rename("old", "new", Some("t1")).await.unwrap();
    secrets
        .delete(&[key("hash", Some("name"), Some("t1"))])
        .await
        .unwrap();
    assert_eq!(fake.calls().len(), calls);
    assert_eq!(teams.lookups.lock().unwrap().is_empty(), calls == 0);
}

#[rstest]
#[case::trailing_slash("litellm/", "litellm/key-a")]
#[case::no_slash("vault", "vault/key-a")]
#[case::empty("", "/key-a")]
#[tokio::test]
async fn prefix_trailing_slash_empty(#[case] prefix: &str, #[case] expected: &str) {
    let config = settings(true, prefix);
    let fake = manager(&[], &[]);
    let teams = FakeTeams::default();
    let secrets = sink(&config, Some(&fake), &teams);
    secrets
        .store("key-a", &SecretString::from("sk-a"), None)
        .await
        .unwrap();
    secrets
        .delete(&[key("hash", Some("key-a"), None)])
        .await
        .unwrap();
    let names: Vec<String> = fake
        .calls()
        .into_iter()
        .map(|call| match call {
            Call::Write { name, .. } | Call::Delete { name, .. } | Call::Read { name, .. } => name,
            Call::Rotate { current, .. } => current,
        })
        .collect();
    assert_eq!(names, [expected, expected]);
}

#[rstest]
#[case::configured(Some("managed"), Some("Team"))]
#[case::defaults(None, None)]
#[tokio::test]
async fn create_raw_value_tags_description(
    #[case] description: Option<&str>,
    #[case] tag: Option<&str>,
) {
    let tags =
        tag.map(|name| SecretTags::Mapping(BTreeMap::from([(name.to_owned(), "core".to_owned())])));
    let config = Settings {
        description: description.map(str::to_owned),
        tags: tags.clone(),
        ..settings(true, "litellm/")
    };
    let fake = manager(&[], &[]);
    let teams = regional_teams();
    let secrets = sink(&config, Some(&fake), &teams);
    secrets
        .store("alias", &SecretString::from("sk-raw-key"), Some("t1"))
        .await
        .unwrap();
    let expected = Call::Write {
        name: "litellm/alias".to_owned(),
        value: "sk-raw-key".to_owned(),
        description: description.map(str::to_owned),
        tags,
        params: Some(object(r#"{"region": "region-eu"}"#)),
    };
    assert_eq!(fake.calls(), [expected]);
}

#[tokio::test]
async fn rotate_both_names_no_creation_options() {
    let config = Settings {
        description: Some("managed".to_owned()),
        tags: Some(SecretTags::Mapping(BTreeMap::new())),
        ..settings(true, "vault")
    };
    let fake = manager(&[], &[]);
    let teams = regional_teams();
    let secrets = sink(&config, Some(&fake), &teams);
    secrets
        .rotate(
            "old-alias",
            "new-alias",
            &SecretString::from("sk-rotated"),
            Some("t2"),
        )
        .await
        .unwrap();
    let expected = Call::Rotate {
        current: "vault/old-alias".to_owned(),
        new: "vault/new-alias".to_owned(),
        value: "sk-rotated".to_owned(),
        params: Some(object(r#"{"region": "region-us"}"#)),
    };
    assert_eq!(fake.calls(), [expected]);
    assert_eq!(*teams.lookups.lock().unwrap(), ["t2"]);
}

#[tokio::test]
async fn rename_read_then_rotate() {
    let config = settings(true, "litellm/");
    let fake = manager(&[("litellm/old", "sk-stored")], &[]);
    let teams = regional_teams();
    let logs = Logs::default();
    let _logging = set_default(tracing_subscriber::registry().with(logs.clone()));
    sink(&config, Some(&fake), &teams)
        .rename("old", "new", Some("t1"))
        .await
        .unwrap();
    let params = Some(object(r#"{"region": "region-eu"}"#));
    let expected = [
        Call::Read {
            name: "litellm/old".to_owned(),
            params: params.clone(),
        },
        Call::Rotate {
            current: "litellm/old".to_owned(),
            new: "litellm/new".to_owned(),
            value: "sk-stored".to_owned(),
            params,
        },
    ];
    assert_eq!(fake.calls(), expected);
    assert_eq!(*teams.lookups.lock().unwrap(), ["t1"]);
    let levels: Vec<Level> = logs.lines().into_iter().map(|(level, _)| level).collect();
    assert_eq!(levels, [Level::INFO]);
}

#[rstest]
#[case::missing(&[], Level::WARN, 1)]
#[case::empty_value(&[("litellm/old", "")], Level::INFO, 2)]
#[tokio::test]
async fn rename_missing_warns_empty_value_rotates(
    #[case] secrets: &[(&str, &str)],
    #[case] level: Level,
    #[case] calls: usize,
) {
    let config = settings(true, "litellm/");
    let fake = manager(secrets, &[]);
    let teams = FakeTeams::default();
    let logs = Logs::default();
    let _logging = set_default(tracing_subscriber::registry().with(logs.clone()));
    sink(&config, Some(&fake), &teams)
        .rename("old", "new", None)
        .await
        .unwrap();
    let recorded = fake.calls();
    assert_eq!(recorded.len(), calls);
    assert_eq!(
        recorded.first(),
        Some(&Call::Read {
            name: "litellm/old".to_owned(),
            params: None
        })
    );
    if calls == 2 {
        let rotate = Call::Rotate {
            current: "litellm/old".to_owned(),
            new: "litellm/new".to_owned(),
            value: String::new(),
            params: None,
        };
        assert_eq!(recorded.get(1), Some(&rotate));
    }
    let lines = logs.lines();
    assert_eq!(
        lines.iter().map(|(found, _)| *found).collect::<Vec<_>>(),
        [level]
    );
    assert!(
        lines
            .iter()
            .all(|(_, message)| message.contains("old") && message.contains("new"))
    );
}

#[tokio::test]
async fn delete_alias_none_vs_empty() {
    let config = settings(true, "litellm/");
    let fake = manager(&[], &[]);
    let teams = FakeTeams::default();
    let logs = Logs::default();
    let _logging = set_default(tracing_subscriber::registry().with(logs.clone()));
    let keys = [
        key("hashed-token-a", None, None),
        key("hashed-token-b", Some(""), None),
    ];
    sink(&config, Some(&fake), &teams)
        .delete(&keys)
        .await
        .unwrap();
    assert_eq!(
        fake.calls(),
        [Call::Delete {
            name: "litellm/".to_owned(),
            window: 7,
            params: None
        }]
    );
    let lines = logs.lines();
    assert_eq!(lines.len(), 1);
    assert!(
        lines
            .iter()
            .all(|(level, message)| *level == Level::WARN && message.contains("hashed-token-a"))
    );
}

#[tokio::test]
async fn delete_team_params_cached() {
    let config = settings(true, "litellm/");
    let fake = manager(&[], &[]);
    let teams = regional_teams();
    let keys = [
        key("a", Some("a"), Some("t1")),
        key("b", Some("b"), Some("t1")),
        key("c", Some("c"), None),
        key("d", Some("d"), Some("t2")),
        key("e", Some("e"), None),
        key("f", Some("f"), Some("t2")),
    ];
    sink(&config, Some(&fake), &teams)
        .delete(&keys)
        .await
        .unwrap();
    assert_eq!(*teams.lookups.lock().unwrap(), ["t1", "t2"]);
    let europe = Some(object(r#"{"region": "region-eu"}"#));
    let america = Some(object(r#"{"region": "region-us"}"#));
    let expected: Vec<Call<Params>> = [
        ("a", &europe),
        ("b", &europe),
        ("c", &None),
        ("d", &america),
        ("e", &None),
        ("f", &america),
    ]
    .into_iter()
    .map(|(alias, params)| Call::Delete {
        name: format!("litellm/{alias}"),
        window: 7,
        params: params.clone(),
    })
    .collect();
    assert_eq!(fake.calls(), expected);
}

#[tokio::test]
async fn delete_failure_stops_later_keys() {
    let config = settings(true, "litellm/");
    let fake = manager(&[], &["litellm/b"]);
    let teams = FakeTeams::default();
    let keys = [
        key("a", Some("a"), None),
        key("b", Some("b"), None),
        key("c", Some("c"), None),
    ];
    let error = sink(&config, Some(&fake), &teams)
        .delete(&keys)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "the secret manager call failed");
    let names: Vec<String> = fake
        .calls()
        .into_iter()
        .map(|call| match call {
            Call::Delete { name, .. } | Call::Write { name, .. } | Call::Read { name, .. } => name,
            Call::Rotate { current, .. } => current,
        })
        .collect();
    assert_eq!(names, ["litellm/a", "litellm/b"]);
}

#[tokio::test]
async fn no_secret_value_in_diagnostics() {
    let config = Settings {
        description: Some("managed".to_owned()),
        ..settings(true, "litellm/")
    };
    let fake = manager(
        &[("litellm/old", "sk-stored-secret")],
        &["litellm/new-alias"],
    );
    let teams = regional_teams();
    let logs = Logs::default();
    let _logging = set_default(tracing_subscriber::registry().with(logs.clone()));
    let secrets = sink(&config, Some(&fake), &teams);
    let value = SecretString::from("sk-raw-secret");
    let failure = secrets
        .store("new-alias", &value, Some("t1"))
        .await
        .unwrap_err();
    secrets.rename("old", "renamed", Some("t1")).await.unwrap();
    secrets
        .delete(&[key("hashed", None, Some("t1"))])
        .await
        .unwrap();
    let diagnostics = format!("{failure} {failure:?} {:?}", logs.lines());
    assert!(!logs.lines().is_empty());
    assert!(!diagnostics.contains("sk-raw-secret"));
    assert!(!diagnostics.contains("sk-stored-secret"));
    assert!(!diagnostics.contains("region-eu"));
}
