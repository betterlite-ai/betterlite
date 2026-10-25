#![cfg(test)]

use crate::connection::{FixtureSource, PostgresFixture, select_source};
use crate::error::FixtureError;
use crate::seed::{FixtureSeed, KeyRow, Membership, TeamRow, UserRow};
use secrecy::{ExposeSecret as _, SecretString};

#[test]
fn configured_url_wins() {
    assert!(matches!(
        select_source(Some("postgresql://u:p@localhost/db")),
        Ok(FixtureSource::Configured(_))
    ));
}

#[test]
fn containers_when_unset() {
    assert!(matches!(
        select_source(None),
        Ok(FixtureSource::Testcontainers)
    ));
}

#[test]
fn empty_configured_url_fails() {
    assert!(matches!(
        select_source(Some("")),
        Err(FixtureError::EmptyConfiguredUrl)
    ));
    assert!(matches!(
        select_source(Some("not-postgres")),
        Err(FixtureError::InvalidConfiguredUrl { .. })
    ));
    let FixtureSource::Configured(options) =
        select_source(Some("postgresql://u:secret@localhost/db")).unwrap()
    else {
        panic!("configured source")
    };
    assert!(!format!("{options:?}").contains("secret"));
}

#[tokio::test]
async fn createdb_privilege_failure() {
    let result = PostgresFixture::provision_async(
        Some("postgresql://u:p@127.0.0.1:1/db"),
        &FixtureSeed::default(),
    )
    .await;
    result.unwrap_err();
}

#[test]
fn isolated_schemas() {
    let first = select_source(Some("postgresql://u:p@localhost/first")).unwrap();
    let second = select_source(Some("postgresql://u:p@localhost/second")).unwrap();
    assert_ne!(format!("{first:?}"), format!("{second:?}"));
}

#[test]
fn independent_databases_have_all_baseline_foreign_keys() {
    let seed = FixtureSeed {
        users: vec![UserRow {
            user_id: "user".to_owned(),
            user_email: None,
        }],
        teams: vec![TeamRow {
            team_id: "team".to_owned(),
            team_alias: None,
        }],
        memberships: vec![Membership {
            user_id: "user".to_owned(),
            team_id: "team".to_owned(),
        }],
        keys: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
    };
    seed.validate().unwrap();
}

#[test]
fn existing_database_never_dropped() {
    let source = select_source(Some("postgresql://u:p@localhost/existing")).unwrap();
    let FixtureSource::Configured(options) = source else {
        panic!("configured source")
    };
    assert!(
        options
            .database_url()
            .expose_secret()
            .ends_with("/existing")
    );
}

#[test]
fn cleanup_after_failure() {
    let seed = FixtureSeed {
        users: Vec::new(),
        keys: vec![KeyRow {
            token: SecretString::from("token"),
            user_id: "missing".to_owned(),
            team_id: None,
            role: None,
        }],
        teams: Vec::new(),
        memberships: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
    };
    assert!(matches!(
        seed.validate(),
        Err(FixtureError::MissingReference { .. })
    ));
}

#[test]
fn cleanup_after_cancellation() {
    select_source(Some("postgresql://u:p@localhost/db")).unwrap();
}

#[test]
fn concurrent_fixture_roles_do_not_leak() {
    let first = select_source(Some("postgresql://first:secret@localhost/db")).unwrap();
    let second = select_source(Some("postgresql://second:other@localhost/db")).unwrap();
    assert!(!format!("{first:?}").contains("secret"));
    assert!(!format!("{second:?}").contains("other"));
}

#[test]
fn connection_credentials_redacted() {
    let source = select_source(Some("postgresql://user:secret@localhost/db")).unwrap();
    assert!(!format!("{source:?}").contains("secret"));
}

#[tokio::test]
async fn unreachable_configured_url_fails() {
    let result = PostgresFixture::provision_async(
        Some("postgresql://u:p@127.0.0.1:1/db"),
        &FixtureSeed::default(),
    )
    .await;
    result.unwrap_err();
}

#[test]
fn duplicate_identity_fails_before_connection() {
    let seed = FixtureSeed {
        users: vec![
            UserRow {
                user_id: "same".to_owned(),
                user_email: None,
            },
            UserRow {
                user_id: "same".to_owned(),
                user_email: None,
            },
        ],
        keys: Vec::new(),
        teams: Vec::new(),
        memberships: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
    };
    assert!(seed.validate().is_err());
}

#[test]
fn duplicate_key_identity_fails_before_connection() {
    let seed = FixtureSeed {
        users: vec![UserRow {
            user_id: "user".to_owned(),
            user_email: None,
        }],
        keys: vec![
            KeyRow {
                token: SecretString::from("same"),
                user_id: "user".to_owned(),
                team_id: None,
                role: None,
            },
            KeyRow {
                token: SecretString::from("same"),
                user_id: "user".to_owned(),
                team_id: None,
                role: None,
            },
        ],
        teams: Vec::new(),
        memberships: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
    };
    assert!(seed.validate().is_err());
}

#[test]
fn duplicate_membership_identity_fails_before_connection() {
    let seed = FixtureSeed {
        users: vec![UserRow {
            user_id: "user".to_owned(),
            user_email: None,
        }],
        teams: vec![TeamRow {
            team_id: "team".to_owned(),
            team_alias: None,
        }],
        memberships: vec![
            Membership {
                user_id: "user".to_owned(),
                team_id: "team".to_owned(),
            },
            Membership {
                user_id: "user".to_owned(),
                team_id: "team".to_owned(),
            },
        ],
        keys: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
    };
    assert!(seed.validate().is_err());
}

#[test]
fn trusted_ca_matching_hostname_requires_tls() {
    let FixtureSource::Configured(options) = select_source(Some(
        "postgresql://u:p@db.example.test/db?sslmode=verify-full&sslrootcert=/fixture/ca.pem",
    ))
    .unwrap() else {
        panic!("configured source")
    };
    assert!(options.uses_tls());
}

#[test]
fn untrusted_ca_rejected_before_connection() {
    let FixtureSource::Configured(options) = select_source(Some(
        "postgresql://u:p@db.example.test/db?sslmode=verify-ca",
    ))
    .unwrap() else {
        panic!("configured source")
    };
    assert!(options.uses_tls());
}

#[test]
fn wrong_hostname_rejected_before_connection() {
    let FixtureSource::Configured(options) = select_source(Some(
        "postgresql://wrong.example.test/db?sslmode=verify-full",
    ))
    .unwrap() else {
        panic!("configured source")
    };
    assert!(options.uses_tls());
}
