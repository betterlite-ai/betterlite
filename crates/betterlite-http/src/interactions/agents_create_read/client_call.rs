use betterlite_config::proxy::ProxyPolicy;
use reqwest::blocking::Client as BlockingClient;

use super::Params;
use crate::interactions::base::ClientError;
use crate::interactions::base::clients::HttpClients;

// the caller's client wins; otherwise the base module resolves one for the params
pub(crate) fn blocking_client<'clients>(
    clients: &'clients mut HttpClients,
    params: &Params,
    supplied: Option<BlockingClient>,
) -> Result<(BlockingClient, Option<&'clients ProxyPolicy>), ClientError> {
    let own_client = supplied.is_none();
    let client = clients.sync_client(params, supplied)?;
    Ok((client, own_client.then_some(&*clients.proxy)))
}
