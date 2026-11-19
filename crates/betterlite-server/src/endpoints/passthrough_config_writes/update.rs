use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::Extension;
use axum::extract::Path;
use axum::extract::State;
use betterlite_db::budget_permission_rows::JsonData;

use crate::auth::auth_identity::AuthContext;
use crate::endpoints::passthrough_config_reads::db_read::PASS_THROUGH_FIELD;
use crate::endpoints::passthrough_config_reads::record::PassThroughEndpointResponse;

use super::convert::stored_value;
use super::error::LookupKind;
use super::error::PassthroughWriteError;
use super::routes::PassThroughWriteState;
use super::routes::decode_body;
use super::routes::install_endpoint;
use super::routes::read_entries;
use super::routes::remove_routes;
use super::routes::require_caller;
use super::routes::require_endpoint;
use super::routes::resolve_headers;
use super::settings_port::ConfigFieldUpdate;
use super::settings_port::GeneralSettingsStore;

pub(super) async fn update_endpoint<Store>(
    State(state): State<Arc<PassThroughWriteState<Store>>>,
    caller: Option<Extension<AuthContext>>,
    Path(endpoint_id): Path<String>,
    body: Bytes,
) -> Result<Json<PassThroughEndpointResponse>, PassthroughWriteError>
where
    Store: GeneralSettingsStore + 'static,
{
    require_caller(caller)?;
    let input = decode_body(&body)?;
    let Some(mut entries) = read_entries(state.settings.as_ref()).await? else {
        return Err(PassthroughWriteError::NoEndpoints {
            kind: LookupKind::Update,
        });
    };
    let (stored, index) = require_endpoint(&entries, &endpoint_id, LookupKind::Update)?;
    let updated = input.merge_into(&stored);
    let headers = resolve_headers(&state, &updated)?;
    let entry = stored_value(&updated)?;
    let Some(slot) = entries.get_mut(index) else {
        return Err(PassthroughWriteError::MissingIndex {
            endpoint_id,
            kind: LookupKind::Update,
        });
    };
    *slot = entry;
    state
        .settings
        .update_field(ConfigFieldUpdate {
            field_name: PASS_THROUGH_FIELD.to_owned(),
            field_value: JsonData::Array(entries),
        })
        .await
        .map_err(PassthroughWriteError::Settings)?;
    remove_routes(&state.routes, &endpoint_id);
    install_endpoint(&state, &endpoint_id, &updated, headers);
    Ok(Json(PassThroughEndpointResponse {
        endpoints: vec![updated],
    }))
}
