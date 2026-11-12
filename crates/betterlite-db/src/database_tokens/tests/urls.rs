#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::database_tokens::settings::TokenToggles;
use crate::database_tokens::strategy::DatabaseTokenError;
use crate::database_tokens::tests::support::{
    AZURE_RAW_TOKEN, FakeAzureFactory, FakeAzureProvider, RecordingRds, azure_providers, providers,
    reader_missing_message, settings, writer_missing_message,
};
use crate::database_tokens::urls::{
    UrlAssemblyError, apply_writer_url_to_env, build_reader_url, build_writer_url,
};
use crate::database_urls::update::ConfigurationUpdate;

fn iam() -> TokenToggles {
    TokenToggles {
        iam_token_db_auth: true,
        azure_postgresql_auth: false,
    }
}

fn azure() -> TokenToggles {
    TokenToggles {
        iam_token_db_auth: false,
        azure_postgresql_auth: true,
    }
}

fn both() -> TokenToggles {
    TokenToggles {
        iam_token_db_auth: true,
        azure_postgresql_auth: true,
    }
}

fn disabled() -> TokenToggles {
    TokenToggles::default()
}

fn update_pairs(updates: &[ConfigurationUpdate]) -> Vec<(String, String)> {
    updates
        .iter()
        .map(|update| (update.name.clone(), update.value.expose_secret().to_owned()))
        .collect()
}

#[test]
fn writer_token_overwrites_pinned_url() {
    let rds = RecordingRds::tokens(&["encoded%3Dtoken"]);
    let token_providers = providers(
        rds.clone(),
        FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN])),
    );
    let database = settings(&[
        ("DATABASE_URL", "postgresql://pinned:secret@other/db"),
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USER", "user"),
        ("DATABASE_NAME", "db"),
        ("IAM_TOKEN_DB_AUTH", "true"),
    ]);
    let url = build_writer_url(&database, iam(), false, &token_providers)
        .expect("the writer assembles")
        .expect("token auth replaces the pinned URL");
    assert_eq!(
        url.expose_secret(),
        "postgresql://user:encoded%3Dtoken@db.example:5432/db"
    );
    assert_eq!(rds.mint_count(), 1);
}

#[test]
fn pooled_writer_skips_mint_after_conflict_check() {
    let rds = RecordingRds::tokens(&["encoded"]);
    let factory = FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN]));
    let token_providers = providers(rds.clone(), factory.clone());
    let database = settings(&[
        ("DATABASE_URL", "postgresql://pooler:secret@pgbouncer/db"),
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USER", "user"),
        ("DATABASE_NAME", "db"),
        ("IAM_TOKEN_DB_AUTH", "true"),
        ("AZURE_POSTGRESQL_AUTH", "true"),
    ]);
    let Err(error) = build_writer_url(&database, both(), true, &token_providers) else {
        panic!("the conflict must beat the pooled marker");
    };
    assert!(matches!(
        error,
        UrlAssemblyError::Token {
            source: DatabaseTokenError::Conflict { .. }
        }
    ));
    assert_eq!(factory.builds(), 0);
    assert_eq!(rds.mint_count(), 0);

    let only_iam = settings(&[
        ("DATABASE_URL", "postgresql://pooler:secret@pgbouncer/db"),
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USER", "user"),
        ("DATABASE_NAME", "db"),
        ("IAM_TOKEN_DB_AUTH", "true"),
    ]);
    assert!(
        build_writer_url(&only_iam, iam(), true, &token_providers)
            .expect("the pooled marker skips minting")
            .is_none()
    );
    assert_eq!(rds.mint_count(), 0);
}

#[rstest]
#[case::all_missing(&[], "DATABASE_HOST, DATABASE_USER, DATABASE_NAME")]
#[case::user_and_name(&[("DATABASE_HOST", "h")], "DATABASE_USER, DATABASE_NAME")]
#[case::name_only(
    &[("DATABASE_HOST", "h"), ("DATABASE_USER", "u")],
    "DATABASE_NAME"
)]
#[case::empty_user(
    &[("DATABASE_HOST", "h"), ("DATABASE_USER", ""), ("DATABASE_NAME", "d")],
    "DATABASE_USER"
)]
fn writer_missing_fields_order_and_message(
    #[case] environment: &[(&str, &str)],
    #[case] fields: &str,
) {
    let expected = format!(
        "IAM_TOKEN_DB_AUTH is enabled but required DB env var(s) are unset: {fields}. Set them so the writer DATABASE_URL can be assembled with a minted RDS IAM token."
    );
    assert_eq!(writer_missing_message(environment), expected);
}

#[test]
fn enabled_flag_update_after_success_only() {
    let token_providers = providers(
        RecordingRds::tokens(&["encoded"]),
        FakeAzureFactory::new(FakeAzureProvider::tokens(&["azure-token"])),
    );
    let database = settings(&[
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USER", "user"),
        ("DATABASE_NAME", "db"),
        ("IAM_TOKEN_DB_AUTH", "true"),
    ]);
    let iam_updates =
        apply_writer_url_to_env(&database, iam(), false, &token_providers).expect("applies");
    assert!(iam_updates.replaced);
    assert_eq!(
        update_pairs(&iam_updates.updates),
        vec![
            (
                "DATABASE_URL".to_owned(),
                "postgresql://user:encoded@db.example:5432/db".to_owned()
            ),
            ("IAM_TOKEN_DB_AUTH".to_owned(), "True".to_owned()),
        ]
    );

    let skipped = apply_writer_url_to_env(&database, iam(), true, &token_providers)
        .expect("the pooled marker skips the update");
    assert!(!skipped.replaced);
    assert!(skipped.updates.is_empty());
}

#[test]
fn azure_flag_update_after_success() {
    let token_providers = providers(
        RecordingRds::tokens(&["encoded"]),
        FakeAzureFactory::new(FakeAzureProvider::tokens(&["azure-token"])),
    );
    let azure_database = settings(&[
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USER", "user"),
        ("DATABASE_NAME", "db"),
        ("AZURE_POSTGRESQL_AUTH", "true"),
    ]);
    let azure_updates = apply_writer_url_to_env(&azure_database, azure(), false, &token_providers)
        .expect("applies");
    assert_eq!(
        update_pairs(&azure_updates.updates),
        vec![
            (
                "DATABASE_URL".to_owned(),
                "postgresql://user:azure-token@db.example:5432/db".to_owned()
            ),
            ("AZURE_POSTGRESQL_AUTH".to_owned(), "True".to_owned()),
        ]
    );
}

#[test]
fn password_update_carries_only_the_url() {
    let token_providers = providers(
        RecordingRds::tokens(&["encoded"]),
        FakeAzureFactory::new(FakeAzureProvider::tokens(&["azure-token"])),
    );
    let password_database = settings(&[
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USER", "user"),
        ("DATABASE_NAME", "db"),
        ("DATABASE_PASSWORD", "secret"),
    ]);
    let password_updates =
        apply_writer_url_to_env(&password_database, disabled(), false, &token_providers)
            .expect("applies");
    assert_eq!(
        update_pairs(&password_updates.updates),
        vec![(
            "DATABASE_URL".to_owned(),
            "postgresql://user:secret@db.example:5432/db".to_owned()
        )]
    );
}

#[test]
fn failed_mint_returns_no_update() {
    let failing = providers(
        RecordingRds::tokens(&[]),
        FakeAzureFactory::new(FakeAzureProvider::tokens(&["azure-token"])),
    );
    let database = settings(&[
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USER", "user"),
        ("DATABASE_NAME", "db"),
        ("IAM_TOKEN_DB_AUTH", "true"),
    ]);
    apply_writer_url_to_env(&database, iam(), false, &failing)
        .expect_err("a failed mint produces no update result");
}

#[test]
fn reader_gates_before_strategy() {
    let rds = RecordingRds::tokens(&["encoded"]);
    let factory = FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN]));
    let token_providers = providers(rds, factory.clone());

    let without_host = settings(&[
        ("IAM_TOKEN_DB_AUTH", "true"),
        ("AZURE_POSTGRESQL_AUTH", "true"),
    ]);
    assert!(
        build_reader_url(&without_host, both(), &token_providers)
            .expect("the reader is opt-in, so no strategy is resolved")
            .is_none()
    );

    let pinned = settings(&[
        ("IAM_TOKEN_DB_AUTH", "true"),
        ("AZURE_POSTGRESQL_AUTH", "true"),
        ("DATABASE_HOST_READ_REPLICA", "reader.example"),
        (
            "DATABASE_URL_READ_REPLICA",
            "postgresql://reader:secret@reader.example/db",
        ),
    ]);
    assert!(
        build_reader_url(&pinned, both(), &token_providers)
            .expect("a pinned reader URL skips the strategy")
            .is_none()
    );
    assert_eq!(factory.builds(), 0);
}

#[test]
fn reader_needs_assembly_before_conflict() {
    let both_enabled = settings(&[
        ("IAM_TOKEN_DB_AUTH", "true"),
        ("AZURE_POSTGRESQL_AUTH", "true"),
        ("DATABASE_HOST_READ_REPLICA", "reader.example"),
    ]);
    let factory = FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN]));
    let token_providers = providers(RecordingRds::default(), factory.clone());
    let Err(error) = build_reader_url(&both_enabled, both(), &token_providers) else {
        panic!("the reader reaches the conflicting strategy");
    };
    assert!(matches!(
        error,
        UrlAssemblyError::Token {
            source: DatabaseTokenError::Conflict { .. }
        }
    ));
    assert_eq!(factory.builds(), 0);
}

#[rstest]
#[case::both_missing(&[], "DATABASE_USER[_READ_REPLICA], DATABASE_NAME[_READ_REPLICA]")]
#[case::name_missing(
    &[("DATABASE_USER_READ_REPLICA", "reader")],
    "DATABASE_NAME[_READ_REPLICA]"
)]
#[case::user_missing(
    &[("DATABASE_NAME_READ_REPLICA", "rdb")],
    "DATABASE_USER[_READ_REPLICA]"
)]
fn reader_missing_fields_exact_message(#[case] environment: &[(&str, &str)], #[case] fields: &str) {
    let expected = format!(
        "AZURE_POSTGRESQL_AUTH is enabled and DATABASE_HOST_READ_REPLICA is set, but the reader could not resolve: {fields} (no *_READ_REPLICA value and no writer fallback). Set the reader fields or the writer defaults."
    );
    assert_eq!(reader_missing_message(environment), expected);
}

#[test]
fn reader_mints_with_pooled_writer() {
    let rds = RecordingRds::tokens(&["encoded"]);
    let token_providers = providers(
        rds.clone(),
        FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN])),
    );
    let database = settings(&[
        ("IAM_TOKEN_DB_AUTH", "true"),
        ("DATABASE_HOST", "writer.example"),
        ("DATABASE_USER", "writer"),
        ("DATABASE_NAME", "wdb"),
        ("DATABASE_PASSWORD", "secret"),
        ("DATABASE_URL", "postgresql://pooler:secret@pgbouncer/wdb"),
        ("DATABASE_HOST_READ_REPLICA", "reader.example"),
        ("DATABASE_PORT_READ_REPLICA", "6543"),
        ("DATABASE_USER_READ_REPLICA", "reader"),
        ("DATABASE_NAME_READ_REPLICA", "rdb"),
    ]);
    assert!(
        build_writer_url(&database, iam(), true, &token_providers)
            .expect("the pooled writer stays as it is")
            .is_none()
    );
    let reader = build_reader_url(&database, iam(), &token_providers)
        .expect("the reader mints")
        .expect("the reader URL is assembled");
    assert_eq!(
        reader.expose_secret(),
        "postgresql://reader:encoded@reader.example:6543/rdb"
    );
    assert_eq!(rds.mint_count(), 1);
}

#[test]
fn writer_reads_writer_fallback_for_reader_fields() {
    let rds = RecordingRds::tokens(&["encoded"]);
    let token_providers = providers(
        rds,
        FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN])),
    );
    let database = settings(&[
        ("IAM_TOKEN_DB_AUTH", "true"),
        ("DATABASE_HOST", "writer.example"),
        ("DATABASE_USER", "writer"),
        ("DATABASE_NAME", "wdb"),
        ("DATABASE_SCHEMA", "tenant"),
        ("DATABASE_HOST_READ_REPLICA", "reader.example"),
    ]);
    let reader = build_reader_url(&database, iam(), &token_providers)
        .expect("the reader mints")
        .expect("the reader falls back to the writer fields");
    assert_eq!(
        reader.expose_secret(),
        "postgresql://writer:encoded@reader.example:5432/wdb?schema=tenant"
    );
}
