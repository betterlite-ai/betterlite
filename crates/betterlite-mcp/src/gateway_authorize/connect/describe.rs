use super::ConnectPorts;
use crate::gateway_authorize::oauth_error;
use crate::gateway_authorize::reply::{ConnectFlowDescription, ConnectFlowState};
use crate::gateway_authorize::scope::{FlowTarget, flow_target};
use crate::gateway_authorize::url_parts::origin_only;
use crate::gateway_dcr::contracts::DB_UNAVAILABLE_DESCRIPTION;
use crate::gateway_dcr::models::ConnectFlow;
use crate::gateway_dcr::oauth_error::OauthError;
use crate::gateway_dcr::oauth_error::OauthErrorCode::TemporarilyUnavailable;
use crate::gateway_dcr::ports::{
    LookupServerReachability, LookupVendorCredential, VendorCredentialState,
};
use crate::server_record::MCPServer;

pub(super) async fn describe_opened_flow<V, R>(
    flow: &ConnectFlow,
    ports: &ConnectPorts<'_, V, R>,
) -> Result<ConnectFlowDescription, OauthError>
where
    V: LookupVendorCredential + ?Sized,
    R: LookupServerReachability + ?Sized,
{
    let target = flow_target(ports.registry, flow, ports.reachability).await;
    let origin = origin_only(&flow.redirect_uri)?;
    match target {
        FlowTarget::Interactive(server) => {
            let connected = match ports
                .vendor
                .lookup(&flow.user_id, &server.fields.server_id)
                .await
            {
                VendorCredentialState::Unavailable => {
                    return Err(oauth_error(
                        503,
                        TemporarilyUnavailable,
                        DB_UNAVAILABLE_DESCRIPTION,
                    ));
                }
                VendorCredentialState::Present => true,
                VendorCredentialState::Absent => false,
            };
            Ok(flow_description(
                ConnectFlowState::Interactive,
                origin,
                Some(server),
                Some(connected),
            ))
        }
        FlowTarget::M2m(server) => Ok(flow_description(
            ConnectFlowState::M2m,
            origin,
            Some(server),
            Some(true),
        )),
        FlowTarget::Unscoped => Ok(flow_description(
            ConnectFlowState::Unscoped,
            origin,
            None,
            None,
        )),
        FlowTarget::Stale => Ok(flow_description(
            ConnectFlowState::Stale,
            origin,
            None,
            None,
        )),
    }
}

fn flow_description(
    state: ConnectFlowState,
    client_origin: String,
    server: Option<&MCPServer>,
    connected: Option<bool>,
) -> ConnectFlowDescription {
    ConnectFlowDescription {
        state,
        client_origin,
        server_id: server.map(|entry| entry.fields.server_id.clone()),
        server_name: server.and_then(server_display_name),
        connected,
    }
}

fn server_display_name(server: &MCPServer) -> Option<String> {
    let fields = &server.fields;
    [
        fields.server_name.as_deref(),
        fields.alias.as_deref(),
        Some(fields.name.as_str()),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.is_empty())
    .map(str::to_owned)
}
