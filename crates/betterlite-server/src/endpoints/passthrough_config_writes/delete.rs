use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::extract::Query;
use axum::extract::State;
use axum::http::Uri;
use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;

use crate::auth::auth_identity::AuthContext;
use crate::endpoints::passthrough_config_reads::db_read::PASS_THROUGH_FIELD;
use crate::endpoints::passthrough_config_reads::record::PassThroughEndpointResponse;

use super::error::LookupKind;
use super::error::PassthroughWriteError;
use super::routes::PassThroughWriteState;
use super::routes::read_entries;
use super::routes::remove_routes;
use super::routes::require_caller;
use super::routes::require_endpoint;
use super::settings_port::ConfigFieldUpdate;
use super::settings_port::GeneralSettingsStore;

#[derive(Deserialize)]
struct DeleteQuery {
    endpoint_id: Option<String>,
}

pub(super) async fn delete_endpoint<Store>(
    State(state): State<Arc<PassThroughWriteState<Store>>>,
    caller: Option<Extension<AuthContext>>,
    uri: Uri,
) -> Result<Json<PassThroughEndpointResponse>, PassthroughWriteError>
where
    Store: GeneralSettingsStore + 'static,
{
    require_caller(caller)?;
    let Ok(Query(query)) = Query::<DeleteQuery>::try_from_uri(&uri) else {
        return Err(PassthroughWriteError::InvalidQuery);
    };
    let Some(endpoint_id) = query.endpoint_id else {
        return Err(PassthroughWriteError::MissingEndpointId);
    };
    let Some(mut entries) = read_entries(state.settings.as_ref()).await? else {
        return Err(PassthroughWriteError::NoEndpoints {
            kind: LookupKind::Delete,
        });
    };
    let (found, index) = require_endpoint(&entries, &endpoint_id, LookupKind::Delete)?;
    entries.remove(index);
    state
        .settings
        .update_field(ConfigFieldUpdate {
            field_name: PASS_THROUGH_FIELD.to_owned(),
            field_value: JsonData::Array(entries),
        })
        .await
        .map_err(PassthroughWriteError::Settings)?;
    remove_routes(&state.routes, &endpoint_id);
    Ok(Json(PassThroughEndpointResponse {
        endpoints: vec![found],
    }))
}
