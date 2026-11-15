#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;

use super::{_get_prisma_client_or_none, get_prisma_client_or_throw};

#[test]
fn a_database_handle_passes_through_both_accessors() {
    let database = 7_u8;
    assert_eq!(_get_prisma_client_or_none(Some(&database)), Some(&database));
    assert_eq!(
        get_prisma_client_or_throw(Some(&database), "Database not available"),
        Ok(&database)
    );
}

#[test]
fn no_database_returns_none_without_raising() {
    assert_eq!(_get_prisma_client_or_none::<u8>(None), None);
}

#[test]
fn a_missing_database_is_a_500_with_the_callers_message() {
    let error = get_prisma_client_or_throw::<u8>(None, "Database not available").unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(error.to_string(), "Database not available");
    assert_eq!(
        error.detail(),
        Some(BTreeMap::from([(
            "error".to_owned(),
            Json::Text("Database not available".to_owned()),
        )]))
    );
}
