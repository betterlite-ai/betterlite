#![cfg(test)]

use std::error::Error as _;

use axum::body::to_bytes;
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse as _;
use betterlite_auth::resolved_key::auth::AuthError;
use betterlite_core::api::vector_store_access::error::VectorStoreAccessError;
use betterlite_db::auxiliary_repository::repository::TableError;

use super::super::caller::CallerView;
use super::super::error::ManagementReadError;
use super::super::list::list_vector_stores;
use super::super::query::pagination;
use super::{FakeDatabase, Setup, World, build, db_store, flags_open, ui_session};

#[tokio::test]
async fn caller_failures_keep_sources_and_response_details() {
    let identity_source = serde_json::from_str::<String>("{").unwrap_err();
    let identity_message = identity_source.to_string();
    let failures = [
        (
            ManagementReadError::Identity {
                source: identity_source,
            },
            identity_message,
            r#"{"detail":"the caller identity could not be read"}"#,
        ),
        (
            ManagementReadError::Key {
                source: AuthError::NonStringApiKey,
            },
            AuthError::NonStringApiKey.to_string(),
            r#"{"detail":"the caller identity could not be built"}"#,
        ),
    ];
    for (error, cause, expected_body) in failures {
        assert_eq!(cause, error.source().unwrap().to_string());
        assert!(!error.is_http_exception());
        let response = error.into_response();
        assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, response.status());
        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(expected_body.as_bytes(), bytes.as_ref());
    }
}

#[test]
fn list_failures_keep_the_original_error_chain() {
    let access_source = serde_json::from_str::<String>("{").unwrap_err();
    let row_source = serde_json::from_str::<String>("{").unwrap_err();
    let cause = access_source.to_string();
    let failures = [
        ManagementReadError::Access {
            source: VectorStoreAccessError::Fields(access_source),
        },
        ManagementReadError::Rows {
            source: TableError::Row(row_source),
        },
    ];
    for error in failures {
        let wrapped = error.source().unwrap();
        assert_eq!(cause, wrapped.source().unwrap().to_string());
        assert_eq!(wrapped.to_string(), error.to_string());
        assert!(!error.is_http_exception());
        assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, error.refusal().0);
    }
}

#[test]
fn invalid_query_keeps_the_integer_parse_error() {
    let uri = Uri::from_static("/vector_store/list?page=bad");
    let error = pagination(&uri).unwrap_err();
    let source = error.source().unwrap();
    let expected = "bad".parse::<i64>().unwrap_err();
    assert_eq!(expected.to_string(), source.to_string());
    assert_eq!(
        "the page query parameter is not an integer",
        error.to_string()
    );
}

#[tokio::test]
async fn list_access_failure_retains_its_source_after_logging() {
    let state = build(Setup {
        database: Some(FakeDatabase::new(vec![db_store("vs_db")])),
        registry: None,
        world: World {
            user_teams: vec!["t1".to_owned()],
            database: true,
        },
        privileges: false,
        settings: flags_open(),
    });
    let caller = CallerView::from_principal(&ui_session("u1").principal).unwrap();
    let error = list_vector_stores(state.as_ref(), &caller, 1, 100)
        .await
        .unwrap_err();
    assert!(error.source().unwrap().is::<VectorStoreAccessError>());
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, error.refusal().0);
    assert!(!error.is_http_exception());
}
