#![cfg(test)]

use std::sync::Arc;

use secrecy::ExposeSecret as _;

use crate::database_tokens::endpoint::IAMEndpoint;
use crate::database_tokens::expiry::TokenAuthKind;
use crate::database_tokens::provider::{AZURE_POSTGRESQL_SCOPE, AzureProviderCache};
use crate::database_tokens::settings::TokenToggles;
use crate::database_tokens::strategy::{
    AzureEntraTokenAuth, CONFLICTING_TOKEN_AUTH_MESSAGE, DatabaseTokenAuth, DatabaseTokenError,
    RdsIamTokenAuth, mint_database_token, resolve,
};
use crate::database_tokens::tests::support::{
    AZURE_RAW_TOKEN, Build, FakeAzureFactory, FakeAzureProvider, RecordingRds, providers, settings,
};
use crate::database_tokens::urls::{UrlAssemblyError, build_writer_url};

fn endpoint() -> IAMEndpoint {
    IAMEndpoint {
        host: "db.example".to_owned(),
        port: "6543".to_owned(),
        user: "svc@corp".to_owned(),
        name: "app db".to_owned(),
        schema: Some("tenant".to_owned()),
    }
}

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

#[test]
fn no_token_strategy() {
    let rds = RecordingRds::tokens(&["ignored"]);
    let factory = FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN]));
    let token_providers = providers(rds.clone(), factory.clone());
    let auth = resolve(TokenToggles::default(), &token_providers).expect("no strategy resolves");
    assert!(auth.is_none());
    assert_eq!(rds.mint_count(), 0);
    assert_eq!(factory.builds(), 0);
}

#[test]
fn conflicting_flags_exact_error_before_provider() {
    let rds = RecordingRds::tokens(&["ignored"]);
    let factory = FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN]));
    let token_providers = providers(rds.clone(), factory.clone());
    let Err(error) = resolve(both(), &token_providers) else {
        panic!("both flags must conflict");
    };
    let expected = "IAM_TOKEN_DB_AUTH and AZURE_POSTGRESQL_AUTH are both enabled, but the database password can only come from one token source. Keep IAM_TOKEN_DB_AUTH for AWS RDS IAM auth, or AZURE_POSTGRESQL_AUTH for Azure Database for PostgreSQL with Microsoft Entra ID, and unset the other one.";
    assert_eq!(error.to_string(), expected);
    assert_eq!(CONFLICTING_TOKEN_AUTH_MESSAGE, expected);
    assert_eq!(factory.builds(), 0);
    assert_eq!(rds.mint_count(), 0);
}

#[test]
fn strategy_labels_and_env_names() {
    let rds_auth: DatabaseTokenAuth<FakeAzureFactory> = DatabaseTokenAuth::Rds(RdsIamTokenAuth {
        provider: Arc::new(RecordingRds::tokens(&[])),
    });
    assert_eq!(rds_auth.label(), "RDS IAM token");
    assert_eq!(rds_auth.env_var(), "IAM_TOKEN_DB_AUTH");
    assert_eq!(rds_auth.kind(), TokenAuthKind::Rds);
    let azure_auth = DatabaseTokenAuth::Azure(AzureEntraTokenAuth {
        providers: Arc::new(AzureProviderCache::new(Arc::new(FakeAzureFactory::new(
            FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN]),
        )))),
    });
    assert_eq!(azure_auth.label(), "Azure Entra token");
    assert_eq!(azure_auth.env_var(), "AZURE_POSTGRESQL_AUTH");
    assert_eq!(azure_auth.kind(), TokenAuthKind::Azure);
}

#[test]
fn azure_scope_ignores_azure_scope_env() {
    let factory = FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN]));
    let token_providers = providers(RecordingRds::default(), factory.clone());
    let auth = resolve(azure(), &token_providers).expect("the Azure strategy resolves");
    assert!(auth.is_some());
    // the factory is handed the PostgreSQL scope explicitly, so an AZURE_SCOPE meant for another service never reaches it
    assert_eq!(factory.scopes(), vec![AZURE_POSTGRESQL_SCOPE.to_owned()]);
}

#[test]
fn azure_provider_cached_once() {
    let factory = FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN]));
    let token_providers = providers(RecordingRds::default(), factory.clone());
    let first = resolve(azure(), &token_providers)
        .expect("the first resolution works")
        .expect("the Azure strategy is enabled");
    let second = resolve(azure(), &token_providers)
        .expect("the second resolution works")
        .expect("the Azure strategy is enabled");
    assert_eq!(factory.builds(), 1);
    let _first_mint = mint_database_token(&first, &endpoint()).expect("the first mint works");
    let _second_mint = mint_database_token(&second, &endpoint()).expect("the second mint works");
    assert_eq!(factory.builds(), 1);
}

#[test]
fn failed_provider_creation_retried() {
    let factory = FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN]));
    factory.queue(Build::Failure);
    factory.queue(Build::Success);
    let token_providers = providers(RecordingRds::default(), factory.clone());
    let Err(error) = resolve(azure(), &token_providers) else {
        panic!("the first construction must fail");
    };
    assert!(matches!(error, DatabaseTokenError::Azure { .. }));
    assert_eq!(factory.builds(), 1);
    let retried = resolve(azure(), &token_providers).expect("the retry succeeds");
    assert!(retried.is_some());
    assert_eq!(factory.builds(), 2);
}

#[test]
fn azure_mint_quotes_fresh_token() {
    let provider = FakeAzureProvider::tokens(&["first/token+one", "second.token~two"]);
    let factory = FakeAzureFactory::new(provider.clone());
    let token_providers = providers(RecordingRds::default(), factory);
    let auth = resolve(azure(), &token_providers)
        .expect("the Azure strategy resolves")
        .expect("the Azure strategy is enabled");
    let first = mint_database_token(&auth, &endpoint()).expect("the first mint works");
    let second = mint_database_token(&auth, &endpoint()).expect("the second mint works");
    assert_eq!(first.expose_secret(), "first%2Ftoken%2Bone");
    assert_eq!(second.expose_secret(), "second.token~two");
    assert_eq!(provider.calls(), 2);
}

#[test]
fn rds_mint_delegates_endpoint() {
    let encoded = "X-Amz-Date%3D20240101T000000Z%26X-Amz-Expires%3D900";
    let rds = RecordingRds::tokens(&[encoded]);
    let token_providers = providers(
        rds.clone(),
        FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN])),
    );
    let auth = resolve(iam(), &token_providers)
        .expect("the RDS strategy resolves")
        .expect("the RDS strategy is enabled");
    let token = mint_database_token(&auth, &endpoint()).expect("the mint works");
    assert_eq!(
        rds.seen(),
        vec![(
            "db.example".to_owned(),
            "6543".to_owned(),
            "svc@corp".to_owned()
        )]
    );
    assert_eq!(token.expose_secret(), encoded);
    assert_eq!(
        endpoint().build_url(token.expose_secret()),
        format!("postgresql://svc%40corp:{encoded}@db.example:6543/app%20db?schema=tenant")
    );
}

#[test]
fn provider_error_no_password_fallback() {
    let provider = FakeAzureProvider::tokens(&[]);
    provider.fail();
    let token_providers = providers(RecordingRds::default(), FakeAzureFactory::new(provider));
    let database = settings(&[
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USER", "user"),
        ("DATABASE_NAME", "db"),
        ("DATABASE_PASSWORD", "secret"),
        ("DATABASE_URL", "postgresql://pinned:secret@other/db"),
        ("AZURE_POSTGRESQL_AUTH", "true"),
    ]);
    let Err(error) = build_writer_url(&database, azure(), false, &token_providers) else {
        panic!("the provider failure must surface");
    };
    assert!(matches!(
        error,
        UrlAssemblyError::Token {
            source: DatabaseTokenError::Azure { .. }
        }
    ));

    let rds_token_providers = providers(
        RecordingRds::tokens(&[]),
        FakeAzureFactory::new(FakeAzureProvider::tokens(&[AZURE_RAW_TOKEN])),
    );
    let Err(rds_error) = build_writer_url(&database, iam(), false, &rds_token_providers) else {
        panic!("the RDS failure must surface");
    };
    assert!(matches!(
        rds_error,
        UrlAssemblyError::Token {
            source: DatabaseTokenError::Rds { .. }
        }
    ));
}
