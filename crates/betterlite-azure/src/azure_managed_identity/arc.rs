use crate::azure_arc_identity::{
    ArcHost, ArcIdentity, ArcTransport, acquire_arc_token, select_arc_endpoint,
};

use super::ManagedIdentityToken;
use super::error::ManagedIdentityError;

pub(super) fn acquire<H>(
    host: &H,
    transport: &dyn ArcTransport,
    client_id: Option<&str>,
    resource: &str,
) -> Result<ManagedIdentityToken, ManagedIdentityError>
where
    H: ArcHost,
{
    let endpoint = select_arc_endpoint(host).ok_or(ManagedIdentityError::ArcEndpoint)?;
    let identity = if client_id.is_some_and(|id| !id.is_empty()) {
        ArcIdentity::UserAssigned
    } else {
        ArcIdentity::SystemAssigned
    };
    let token = acquire_arc_token(host, &transport, &endpoint, resource, identity)?;
    Ok(ManagedIdentityToken {
        token: token.token,
        expires_on: Some(token.expires_on),
        token_type: Some(token.token_type),
        refresh_on: token.refresh_on,
    })
}
