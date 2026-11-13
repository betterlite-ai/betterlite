use std::sync::{Arc, Mutex};

use betterlite_azure::azure_client_secret::DEFAULT_AUTHORITY_HOST;
use betterlite_azure::azure_client_secret::transport::Clock as TokenClock;
use betterlite_azure::azure_default_chain::grant::{GrantError, UsernamePasswordRecord};
use betterlite_azure::azure_default_chain::native::NativeGrant;
use betterlite_azure::azure_default_chain::native::password::{EnvironmentPasswordGrant, State};
use betterlite_azure::azure_username_password::UsernamePasswordInput;
use betterlite_azure::azure_username_password::transport::Transport as PasswordTransport;

use super::super::azure_http::AzureIo;
use super::super::azure_production::SdkAzureGrants;
use super::super::azure_runtime::ProductionHost;

pub(super) fn password_grant<'grants, H: AzureIo>(
    grants: &'grants SdkAzureGrants<H>,
    record: &UsernamePasswordRecord,
) -> Result<NativeGrant<'grants, ProductionHost>, GrantError> {
    let tenant = record
        .tenant_id
        .as_deref()
        .filter(|tenant| !tenant.is_empty())
        .map_or_else(|| "organizations".to_owned(), str::to_owned);
    let transport: &dyn PasswordTransport = &*grants.http;
    let authority_host = grants
        .authority_host
        .clone()
        .unwrap_or_else(|| DEFAULT_AUTHORITY_HOST.to_owned());
    let input = UsernamePasswordInput {
        client_id: record.client_id.clone(),
        username: record.username.clone(),
        password: record.password.clone(),
        scope: Some(record.scope.clone()),
    };
    let clock: Arc<dyn TokenClock> = Arc::clone(&grants.clock);
    let grant = EnvironmentPasswordGrant {
        input,
        tenant,
        authority_host,
        transport,
        clock,
        state: Mutex::new(State::default()),
    };
    Ok(NativeGrant::UsernamePassword(grant))
}
