use betterlite_auth::auth_errors::conversion::as_proxy_exception;
use betterlite_auth::auth_key_lookup::error::KeyLookupError;
use betterlite_auth::identity_store::error::IdentityStoreError;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorInput {
    kind: ErrorKind,
    #[serde(default)]
    hashed_token: String,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum ErrorKind {
    MissingKey,
    NoDatabase,
    NotInCache,
    MissingSourceKey,
}

impl ErrorKind {
    const fn label(self) -> &'static str {
        match self {
            Self::MissingKey => "missing_key",
            Self::NoDatabase => "no_database",
            Self::NotInCache => "not_in_cache",
            Self::MissingSourceKey => "missing_source_key",
        }
    }
}

pub(super) fn error(input: &ErrorInput) -> HelperResult {
    let failure = store_error(input).failure();
    let client = as_proxy_exception(&failure);
    let exception = client.exception();
    Ok(json!({
        "kind": input.kind.label(),
        "message": &exception.message,
        "type": &exception.error_type,
        "param": exception.param.as_deref(),
        "code": &exception.code,
    }))
}

fn store_error(input: &ErrorInput) -> IdentityStoreError {
    match input.kind {
        ErrorKind::MissingKey => IdentityStoreError::KeyLookup(KeyLookupError::MissingKey {
            key: input.hashed_token.clone(),
        }),
        ErrorKind::NoDatabase => IdentityStoreError::KeyLookup(KeyLookupError::NoDatabase),
        ErrorKind::NotInCache => IdentityStoreError::KeyLookup(KeyLookupError::NotInCache {
            key: input.hashed_token.clone(),
        }),
        ErrorKind::MissingSourceKey => IdentityStoreError::PrincipalMissingSourceKey,
    }
}
