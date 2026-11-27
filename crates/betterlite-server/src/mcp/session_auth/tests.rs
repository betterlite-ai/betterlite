#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::SessionAuth;
use crate::mcp::auth_user::McpAuthFields;

const SHARED_HEADER: &str = "test-shared-header";
const SERVER_HEADER: &str = "test-server-header";
const OAUTH2_HEADER: &str = "test-oauth2-header";
const RAW_HEADER: &str = "test-raw-header";
const NEXT_SHARED_HEADER: &str = "test-next-shared-header";
const NEXT_SERVER_HEADER: &str = "test-next-server-header";
const NEXT_OAUTH2_HEADER: &str = "test-next-oauth2-header";
const NEXT_RAW_HEADER: &str = "test-next-raw-header";
const SERVER: &str = "server-a";
const NEXT_SERVER: &str = "server-b";
const CLIENT_IP: &str = "192.0.2.10";
const NEXT_CLIENT_IP: &str = "198.51.100.20";
const PROTOCOL_VERSION: &str = "2025-06-18";
const NEXT_PROTOCOL_VERSION: &str = "2025-11-25";
const CALLER: &str = "test-caller-a";
const NEXT_CALLER: &str = "test-caller-b";

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn principal(caller: &str) -> UserAPIKeyAuth {
    let fields = BTreeMap::from([("key_alias".to_owned(), JsonData::String(caller.to_owned()))]);
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("fixture should construct")
}

fn authorization(value: &str) -> BTreeMap<String, SecretString> {
    BTreeMap::from([("Authorization".to_owned(), secret(value))])
}

fn server_headers(server: &str, value: &str) -> BTreeMap<String, BTreeMap<String, SecretString>> {
    BTreeMap::from([(server.to_owned(), authorization(value))])
}

fn populated_fields() -> McpAuthFields {
    McpAuthFields {
        user_api_key_auth: Some(principal(CALLER)),
        mcp_auth_header: Some(secret(SHARED_HEADER)),
        mcp_servers: Some(vec![SERVER.to_owned()]),
        mcp_server_auth_headers: Some(server_headers(SERVER, SERVER_HEADER)),
        oauth2_headers: Some(authorization(OAUTH2_HEADER)),
        mcp_protocol_version: Some(PROTOCOL_VERSION.to_owned()),
        raw_headers: Some(authorization(RAW_HEADER)),
        client_ip: Some(CLIENT_IP.to_owned()),
    }
}

fn next_caller_fields() -> McpAuthFields {
    let mut fields = populated_fields();
    fields.user_api_key_auth = Some(principal(NEXT_CALLER));
    fields.mcp_auth_header = Some(secret(NEXT_SHARED_HEADER));
    fields.mcp_servers = Some(vec![NEXT_SERVER.to_owned()]);
    fields.mcp_server_auth_headers = Some(server_headers(NEXT_SERVER, NEXT_SERVER_HEADER));
    fields.oauth2_headers = Some(authorization(NEXT_OAUTH2_HEADER));
    fields.mcp_protocol_version = Some(NEXT_PROTOCOL_VERSION.to_owned());
    fields.raw_headers = Some(authorization(NEXT_RAW_HEADER));
    fields.client_ip = Some(NEXT_CLIENT_IP.to_owned());
    fields
}

#[test]
fn new_handle_reads_missing_server_headers_as_an_empty_map() {
    let auth = SessionAuth::new(McpAuthFields::default());
    let snapshot = auth.snapshot();
    assert!(snapshot.mcp_server_auth_headers.is_empty());
    assert!(snapshot.user_api_key_auth.is_none());
    assert!(snapshot.mcp_protocol_version.is_none());
}

#[test]
fn new_handle_is_a_fresh_value_each_time() {
    let first = SessionAuth::new(populated_fields());
    let second = SessionAuth::new(next_caller_fields());
    assert!(!Arc::ptr_eq(&first.user, &second.user));
}

#[test]
fn snapshot_returns_the_caller_values() {
    let auth = SessionAuth::new(populated_fields());
    let snapshot = auth.snapshot();
    assert_eq!(
        format!("{:?}", snapshot.user_api_key_auth),
        format!("{:?}", Some(principal(CALLER)))
    );
    assert_eq!(
        snapshot
            .mcp_auth_header
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(SHARED_HEADER)
    );
    assert_eq!(snapshot.mcp_servers, Some(vec![SERVER.to_owned()]));
}

#[test]
fn snapshot_returns_the_header_maps() {
    let auth = SessionAuth::new(populated_fields());
    let snapshot = auth.snapshot();
    assert_eq!(
        snapshot
            .mcp_server_auth_headers
            .get(SERVER)
            .and_then(|headers| headers.get("Authorization"))
            .map(ExposeSecret::expose_secret),
        Some(SERVER_HEADER)
    );
    assert_eq!(
        snapshot
            .oauth2_headers
            .as_ref()
            .and_then(|headers| headers.get("Authorization"))
            .map(ExposeSecret::expose_secret),
        Some(OAUTH2_HEADER)
    );
    assert_eq!(
        snapshot
            .raw_headers
            .as_ref()
            .and_then(|headers| headers.get("Authorization"))
            .map(ExposeSecret::expose_secret),
        Some(RAW_HEADER)
    );
}

#[test]
fn snapshot_returns_the_client_ip_and_protocol_version() {
    let auth = SessionAuth::new(populated_fields());
    let snapshot = auth.snapshot();
    assert_eq!(snapshot.client_ip.as_deref(), Some(CLIENT_IP));
    assert_eq!(
        snapshot.mcp_protocol_version.as_deref(),
        Some(PROTOCOL_VERSION)
    );
}

#[test]
fn a_snapshot_keeps_the_values_it_read_before_a_later_update() {
    let auth = SessionAuth::new(populated_fields());
    let before = auth.snapshot();
    let updated = SessionAuth::set_or_update(Some(&auth), next_caller_fields(), false);
    assert_eq!(
        updated.snapshot().mcp_servers,
        Some(vec![NEXT_SERVER.to_owned()])
    );
    assert_eq!(before.mcp_servers, Some(vec![SERVER.to_owned()]));
    assert_eq!(
        before
            .mcp_auth_header
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(SHARED_HEADER)
    );
}

#[rstest]
#[case::mcp_auth_header(SHARED_HEADER)]
#[case::server_auth_header(SERVER_HEADER)]
#[case::oauth2_header(OAUTH2_HEADER)]
#[case::raw_header(RAW_HEADER)]
fn debug_never_prints_a_header_value(#[case] hidden: &str) {
    let printed = format!("{:?}", SessionAuth::new(populated_fields()));
    assert!(printed.contains("SessionAuth"));
    assert!(!printed.contains(hidden));
}

#[test]
fn debug_prints_the_values_that_are_not_credentials() {
    let printed = format!("{:?}", SessionAuth::new(populated_fields()));
    assert!(printed.contains(CLIENT_IP));
    assert!(printed.contains(PROTOCOL_VERSION));
}

#[test]
fn set_or_update_overwrites_the_caller_fields_in_place() {
    let auth = SessionAuth::new(populated_fields());
    let updated = SessionAuth::set_or_update(Some(&auth), next_caller_fields(), false);
    assert!(Arc::ptr_eq(&updated.user, &auth.user));
    let snapshot = auth.snapshot();
    assert_eq!(
        format!("{:?}", snapshot.user_api_key_auth),
        format!("{:?}", Some(principal(NEXT_CALLER)))
    );
    assert_eq!(
        snapshot
            .mcp_auth_header
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(NEXT_SHARED_HEADER)
    );
    assert_eq!(snapshot.mcp_servers, Some(vec![NEXT_SERVER.to_owned()]));
    assert_eq!(
        snapshot.mcp_protocol_version.as_deref(),
        Some(PROTOCOL_VERSION)
    );
}

#[test]
fn set_or_update_overwrites_the_header_maps_and_client_ip() {
    let auth = SessionAuth::new(populated_fields());
    let updated = SessionAuth::set_or_update(Some(&auth), next_caller_fields(), false);
    let snapshot = updated.snapshot();
    assert_eq!(
        snapshot
            .mcp_server_auth_headers
            .get(NEXT_SERVER)
            .and_then(|headers| headers.get("Authorization"))
            .map(ExposeSecret::expose_secret),
        Some(NEXT_SERVER_HEADER)
    );
    assert!(snapshot.mcp_server_auth_headers.get(SERVER).is_none());
    assert_eq!(
        snapshot
            .oauth2_headers
            .as_ref()
            .and_then(|headers| headers.get("Authorization"))
            .map(ExposeSecret::expose_secret),
        Some(NEXT_OAUTH2_HEADER)
    );
    assert_eq!(
        snapshot
            .raw_headers
            .as_ref()
            .and_then(|headers| headers.get("Authorization"))
            .map(ExposeSecret::expose_secret),
        Some(NEXT_RAW_HEADER)
    );
    assert_eq!(snapshot.client_ip.as_deref(), Some(NEXT_CLIENT_IP));
}

#[test]
fn set_or_update_clears_fields_the_new_caller_omits() {
    let auth = SessionAuth::new(populated_fields());
    let updated = SessionAuth::set_or_update(Some(&auth), McpAuthFields::default(), false);
    let snapshot = updated.snapshot();
    assert!(snapshot.user_api_key_auth.is_none());
    assert!(snapshot.mcp_auth_header.is_none());
    assert!(snapshot.mcp_servers.is_none());
    assert!(snapshot.mcp_server_auth_headers.is_empty());
    assert!(snapshot.oauth2_headers.is_none());
    assert!(snapshot.raw_headers.is_none());
    assert!(snapshot.client_ip.is_none());
    assert_eq!(
        snapshot.mcp_protocol_version.as_deref(),
        Some(PROTOCOL_VERSION)
    );
}

#[test]
fn set_or_update_keeps_the_protocol_version_the_handle_already_held() {
    let auth = SessionAuth::new(populated_fields());
    let updated = SessionAuth::set_or_update(Some(&auth), next_caller_fields(), false);
    assert_eq!(
        updated.snapshot().mcp_protocol_version.as_deref(),
        Some(PROTOCOL_VERSION)
    );
}

#[test]
fn set_or_update_writes_through_every_clone_of_the_handle() {
    let auth = SessionAuth::new(populated_fields());
    let shared = auth.clone();
    let updated = SessionAuth::set_or_update(Some(&shared), next_caller_fields(), false);
    assert!(Arc::ptr_eq(&updated.user, &auth.user));
    assert_eq!(
        auth.snapshot().mcp_servers,
        Some(vec![NEXT_SERVER.to_owned()])
    );
}

#[test]
fn set_or_update_with_copy_existing_builds_a_new_handle() {
    let auth = SessionAuth::new(populated_fields());
    let copied = SessionAuth::set_or_update(Some(&auth), next_caller_fields(), true);
    assert!(!Arc::ptr_eq(&copied.user, &auth.user));
    let old = auth.snapshot();
    assert_eq!(old.mcp_servers, Some(vec![SERVER.to_owned()]));
    assert_eq!(
        old.mcp_auth_header
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(SHARED_HEADER)
    );
    assert_eq!(old.client_ip.as_deref(), Some(CLIENT_IP));
    let fresh = copied.snapshot();
    assert_eq!(fresh.mcp_servers, Some(vec![NEXT_SERVER.to_owned()]));
    assert_eq!(fresh.client_ip.as_deref(), Some(NEXT_CLIENT_IP));
    assert_eq!(
        fresh.mcp_protocol_version.as_deref(),
        Some(NEXT_PROTOCOL_VERSION)
    );
}

#[test]
fn set_or_update_without_a_handle_builds_a_new_handle() {
    let created = SessionAuth::set_or_update(None, populated_fields(), false);
    let snapshot = created.snapshot();
    assert_eq!(snapshot.mcp_servers, Some(vec![SERVER.to_owned()]));
    assert_eq!(
        snapshot
            .mcp_auth_header
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(SHARED_HEADER)
    );
    assert_eq!(snapshot.client_ip.as_deref(), Some(CLIENT_IP));
}
