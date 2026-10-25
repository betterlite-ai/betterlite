#![cfg(test)]

use crate::connection::{FixtureSource, PostgresFixture, select_source};
use crate::error::FixtureError;
use crate::seed::{FixtureSeed, KeyRow, Membership, TeamRow, UserRow};
use secrecy::SecretString;

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
fn empty_or_malformed_urls_and_credentials_are_handled() {
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
