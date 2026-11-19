use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::Extension;
use axum::extract::State;
use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::auth::auth_identity::AuthContext;
use crate::endpoints::passthrough_config_reads::db_read::PASS_THROUGH_FIELD;
use crate::endpoints::passthrough_config_reads::record::EndpointFlag;
use crate::endpoints::passthrough_config_reads::record::PassThroughEndpointResponse;
use crate::endpoints::passthrough_config_reads::record::PassThroughGenericEndpoint;

use super::convert::stored_value;
use super::error::PassthroughWriteError;
use super::routes::PassThroughWriteState;
use super::routes::decode_body;
use super::routes::install_endpoint;
use super::routes::read_entries;
use super::routes::require_caller;
use super::routes::resolve_headers;
use super::settings_port::ConfigFieldUpdate;
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
    // LiteLLM's create is the body model with its defaults, and an absent id gets a UUID v4
    let created = input.merge_into(&PassThroughGenericEndpoint {
        id: input.id.is_none().then(|| Uuid::new_v4().to_string()),
        path: String::new(),
        target: String::new(),
        headers: BTreeMap::new(),
        default_query_params: BTreeMap::new(),
        include_subpath: EndpointFlag(false),
        cost_per_request: Decimal::ZERO,
        timeout: None,
        auth: EndpointFlag(true),
        guardrails: None,
        is_from_config: EndpointFlag(false),
        methods: None,
    });
    let headers = resolve_headers(&state, &created)?;
    let entry = stored_value(&created)?;
    entries.push(entry);
    state
        .settings
        .update_field(ConfigFieldUpdate {
            field_name: PASS_THROUGH_FIELD.to_owned(),
            field_value: JsonData::Array(entries),
        })
        .await
        .map_err(PassthroughWriteError::Settings)?;
    install_endpoint(&state, "", &created, headers);
    Ok(Json(PassThroughEndpointResponse {
        endpoints: vec![created],
    }))
}
