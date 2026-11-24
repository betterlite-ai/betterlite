use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::gateway_dcr::contracts::{GATEWAY_DCR_CLIENT_ID_PREFIX, MAX_CLIENT_ID_LENGTH};
use betterlite_mcp::gateway_dcr::oauth_error::{OauthError, OauthErrorResponse as _};
use betterlite_mcp::gateway_dcr::registration::{
    RegisteredClient, Registration, register_aggregate_client,
};
use betterlite_mcp::gateway_dcr::sealed::open_gateway_dcr_client;
use betterlite_mcp::redirect_trust::shape::native_callbacks;
use betterlite_oracle_helper_api::error::HelperError;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::{RegisterCall, failure};

pub(super) fn reports(
    calls: &[RegisterCall],
    cipher: &DbCipher,
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut reports = Vec::with_capacity(calls.len());
    for call in calls {
        reports.push(report(call, cipher)?);
    }
    Ok(reports)
}

fn report(call: &RegisterCall, cipher: &DbCipher) -> Result<Value, Box<HelperError>> {
    let Some(now) = DateTime::<Utc>::from_timestamp(call.now, 0) else {
        return Err(failure("now is not a valid unix timestamp"));
    };
    let configured = call.native_redirect_uris.as_deref().unwrap_or_default();
    let callbacks = native_callbacks(configured);
    let registration = register_aggregate_client(
        &call.body,
        call.token_exchange_available,
        now,
        cipher,
        &callbacks,
    );
    Ok(match registration {
        Registration::Registered(client) => success_report(&client, cipher),
        Registration::Refused(error) => error_report(&error),
    })
}

fn success_report(client: &RegisteredClient, cipher: &DbCipher) -> Value {
    let opened = open_gateway_dcr_client(&client.client_id, cipher);
    let matches = opened
        .as_ref()
        .is_some_and(|record| record.iat == client.client_id_issued_at);
    let opened_uris = opened.map(|record| record.redirect_uris);
    json!({
        "status": 201_u16,
        "error": null,
        "error_description": null,
        "cache_headers": [],
        "client_id_prefix_ok": client.client_id.starts_with(GATEWAY_DCR_CLIENT_ID_PREFIX),
        "client_id_within_cap": client.client_id.chars().count() <= MAX_CLIENT_ID_LENGTH,
        "issued_at_matches_client_id": matches,
        "client_id_issued_at": client.client_id_issued_at,
        "redirect_uris": client.redirect_uris,
        "opened_redirect_uris": opened_uris,
        "token_endpoint_auth_method": client.token_endpoint_auth_method,
        "grant_types": client.grant_types,
        "response_types": client.response_types,
    })
}

fn error_report(error: &OauthError) -> Value {
    let mut headers: Vec<[String; 2]> = error
        .headers()
        .into_iter()
        .map(|(name, value)| [name.to_lowercase(), value.to_owned()])
        .collect();
    headers.sort();
    json!({
        "status": error.status,
        "error": error.error.as_str(),
        "error_description": error.description,
        "cache_headers": headers,
        "client_id_prefix_ok": null,
        "client_id_within_cap": null,
        "issued_at_matches_client_id": null,
        "client_id_issued_at": null,
        "redirect_uris": null,
        "opened_redirect_uris": null,
        "token_endpoint_auth_method": null,
        "grant_types": null,
        "response_types": null,
    })
}
