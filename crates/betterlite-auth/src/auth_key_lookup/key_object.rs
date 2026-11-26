use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::combined_key_read::view::CombinedKeyView;
use opentelemetry::trace::SpanContext;
use tracing::debug;

use super::KeyLookups;
use super::error::KeyLookupError;
use crate::auth_cache::copy::copy_for_cache;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

// get_key_object's arguments: the supplied hash is already the cache key, so it is never hashed here.
pub struct KeyRequest<'request> {
    pub hashed_token: &'request str,
    pub parent_otel_span: Option<SpanContext>,
    pub check_cache_only: Option<bool>,
    pub check_db_only: bool,
}

pub async fn get_key_object(
    lookups: &KeyLookups,
    request: &KeyRequest<'_>,
) -> Result<UserAPIKeyAuth, KeyLookupError> {
    let Some(reader) = lookups.reader.as_ref() else {
        return Err(KeyLookupError::NoDatabase);
    };
    if !request.check_db_only
        && let Some(cached) = lookups.cache.read_key(request.hashed_token).await
    {
        return Ok(copy_for_cache(&cached));
    }
    if request.check_cache_only.unwrap_or(false) {
        return Err(KeyLookupError::NotInCache {
            key: request.hashed_token.to_owned(),
        });
    }
    let Some(view) = reader
        .fetch(
            request.hashed_token,
            request.check_db_only,
            request.parent_otel_span.clone(),
        )
        .await?
    else {
        return Err(KeyLookupError::MissingKey {
            key: request.hashed_token.to_owned(),
        });
    };
    let mut auth = auth_from_view(view)?;
    enrich_permission(lookups, &mut auth, request).await?;
    if request.check_db_only {
        return Ok(auth);
    }
    lookups
        .writer
        .write_key(request.hashed_token, &mut auth)
        .await
        .map_err(|error| KeyLookupError::CacheWrite(Box::new(error)))?;
    Ok(auth)
}

// UserAPIKeyAuth.model_validate(row.model_dump(exclude_none=True)): a null row field reads as its default.
fn auth_from_view(view: CombinedKeyView) -> Result<UserAPIKeyAuth, KeyLookupError> {
    let text = serde_json::to_string(&view).map_err(KeyLookupError::Json)?;
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(&text).map_err(KeyLookupError::Json)?;
    UserAPIKeyAuth::construct(AuthInput::Dict(fields))
        .map_err(|error| KeyLookupError::Row(Box::new(error)))
}

// The relation is loaded when an id exists and it is absent, and always on a writer read.
async fn enrich_permission(
    lookups: &KeyLookups,
    auth: &mut UserAPIKeyAuth,
    request: &KeyRequest<'_>,
) -> Result<(), KeyLookupError> {
    let Some(object_permission_id) = auth.view.token.object_permission_id.as_deref() else {
        return Ok(());
    };
    if !request.check_db_only && auth.view.token.object_permission.is_some() {
        return Ok(());
    }
    let Some(permissions) = lookups.permissions.as_deref() else {
        return Ok(());
    };
    match permissions
        .permission(object_permission_id, request.check_db_only)
        .await
    {
        Ok(permission) => {
            auth.view.token.object_permission = permission;
            Ok(())
        }
        Err(error) if request.check_db_only => Err(KeyLookupError::Permission(error)),
        Err(error) => {
            debug!(
                object_permission_id,
                error = %error,
                "Failed to load object_permission for key"
            );
            Ok(())
        }
    }
}
