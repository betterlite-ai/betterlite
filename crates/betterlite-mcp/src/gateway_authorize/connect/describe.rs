use betterlite_db::stored_cipher::DbCipher;

use super::{ConnectFlowCall, ConnectPorts, open_flow_for};
use crate::gateway_authorize::no_cache_headers;
use crate::gateway_authorize::reply::GatewayReply;
use crate::gateway_dcr::oauth_error::OauthError;
use crate::gateway_dcr::ports::{LookupServerReachability, LookupVendorCredential};

pub(super) async fn described_reply<V, R>(
    call: &ConnectFlowCall<'_>,
    ports: &ConnectPorts<'_, V, R>,
    cipher: &DbCipher,
) -> Result<GatewayReply, OauthError>
where
    V: LookupVendorCredential + ?Sized,
    R: LookupServerReachability + ?Sized,
{
    let flow = open_flow_for(
        call.view,
        call.handle,
        call.session_user_id,
        call.now,
        cipher,
    )?;
    let body = Box::new(super::describe_opened_flow(&flow, ports).await?);
    Ok(GatewayReply::Json {
        body,
        headers: no_cache_headers(),
    })
}
