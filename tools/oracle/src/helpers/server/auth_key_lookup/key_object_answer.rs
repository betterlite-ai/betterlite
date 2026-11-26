use betterlite_auth::auth_key_lookup::error::{KeyLookupError, proxy_exception};
use betterlite_auth::auth_key_lookup::key_object::{KeyRequest, get_key_object};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};

use super::KeyObjectInput;
use super::fixtures::read_fixture;
use super::token_fields;

pub(super) async fn key_object(input: &KeyObjectInput) -> HelperResult {
    let fixture = read_fixture(input)?;
    let lookups = fixture.lookups();
    let request = KeyRequest {
        hashed_token: &input.hashed_token,
        parent_otel_span: None,
        check_cache_only: input.check_cache_only,
        check_db_only: input.check_db_only,
    };
    let (outcome, fields, error) = match get_key_object(&lookups, &request).await {
        Ok(auth) => ("ok", token_fields(&auth)?, Value::Null),
        Err(failure) => (outcome_label(&failure), Value::Null, error_report(&failure)),
    };
    Ok(json!({
        "outcome": outcome,
        "error": error,
        "fields": fields,
        "use_writer": fixture.source.calls(),
        "cache_written": fixture.writer.writes(),
        "permission_calls": fixture.permission_modes(),
        "reconnects": fixture.reconnect_calls(),
    }))
}

fn outcome_label(error: &KeyLookupError) -> &'static str {
    match error {
        KeyLookupError::MissingKey { .. } => "missing_key",
        KeyLookupError::NotInCache { .. } => "not_in_cache",
        KeyLookupError::Permission(_) => "permission",
        KeyLookupError::NoDatabase
        | KeyLookupError::Read(_)
        | KeyLookupError::Lookup(_)
        | KeyLookupError::Row(_)
        | KeyLookupError::NotAnObject
        | KeyLookupError::Json(_)
        | KeyLookupError::CacheWrite(_) => "database",
    }
}

fn error_report(error: &KeyLookupError) -> Value {
    match error {
        KeyLookupError::MissingKey { .. } => {
            let exception = proxy_exception(error);
            json!({
                "type": exception.error_type,
                "param": exception.param,
                "code": exception.code,
            })
        }
        KeyLookupError::NoDatabase
        | KeyLookupError::NotInCache { .. }
        | KeyLookupError::Read(_)
        | KeyLookupError::Lookup(_)
        | KeyLookupError::Row(_)
        | KeyLookupError::NotAnObject
        | KeyLookupError::Json(_)
        | KeyLookupError::Permission(_)
        | KeyLookupError::CacheWrite(_) => Value::Null,
    }
}
