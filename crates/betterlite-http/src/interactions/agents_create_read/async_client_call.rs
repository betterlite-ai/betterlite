use betterlite_config::proxy::ProxyPolicy;
use reqwest::Client;

use super::Params;
use crate::interactions::base::ClientError;
use crate::interactions::base::clients::{self, HttpClients};

// the caller's client wins; otherwise the base module resolves one for the named provider
pub(crate) fn async_client<'clients>(
    pool: &'clients mut HttpClients,
    params: &Params,
    supplied: Option<Client>,
) -> Result<(Client, Option<&'clients ProxyPolicy>), ClientError> {
    let own_client = supplied.is_none();
    let client = clients::async_client(pool, params, supplied)?;
    Ok((client, own_client.then_some(&*pool.proxy)))
}
