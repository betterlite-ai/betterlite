use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::Extension;
use axum::extract::State;

use crate::auth::auth_identity::AuthContext;
use crate::endpoints::passthrough_config_reads::record::PassThroughEndpointResponse;

use super::convert::stored_value;
use super::error::PassthroughWriteError;
use super::routes::PassThroughWriteState;
use super::routes::decode_body;
use super::routes::install_endpoint;
use super::routes::read_entries;
use super::routes::require_caller;
use super::routes::resolve_headers;
use super::routes::store_entries;
use super::settings_port::GeneralSettingsStore;

pub(super) async fn create_endpoint<Store>(
    State(state): State<Arc<PassThroughWriteState<Store>>>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<PassThroughEndpointResponse>, PassthroughWriteError>
where
    Store: GeneralSettingsStore + 'static,
{
    require_caller(caller)?;
    let input = decode_body(&body)?;
    let mut entries = read_entries(state.settings.as_ref())
        .await?
        .unwrap_or_default();
    let created = input.into_endpoint();
    let headers = resolve_headers(&state, &created)?;
    let entry = stored_value(&created)?;
    entries.push(entry);
    store_entries(state.settings.as_ref(), entries).await?;
    install_endpoint(&state, "", &created, headers);
    Ok(Json(PassThroughEndpointResponse {
        endpoints: vec![created],
    }))
}
