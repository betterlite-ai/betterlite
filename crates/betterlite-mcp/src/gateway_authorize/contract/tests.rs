#![cfg(test)]

use super::*;

const BASE: &str = "https://proxy.test:8443";
const TOKEN_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";

#[test]
fn the_contract_lists_the_gateway_endpoints() {
    let contract = native_client_auth_contract(BASE, false);
    assert_eq!(
        serde_json::to_string(&contract).unwrap(),
        concat!(
            r#"{"contract_version":1,"issuer":"https://proxy.test:8443","#,
            r#""authorization_endpoint":"https://proxy.test:8443/authorize","#,
            r#""token_endpoint":"https://proxy.test:8443/token","#,
            r#""registration_endpoint":"https://proxy.test:8443/register","#,
            r#""revocation_endpoint":"https://proxy.test:8443/revoke","#,
            r#""resource":"https://proxy.test:8443","#,
            r#""response_types_supported":["code"],"#,
            r#""grant_types_supported":["authorization_code","refresh_token"],"#,
            r#""code_challenge_methods_supported":["S256"],"#,
            r#""token_endpoint_auth_methods_supported":["none"],"#,
            r#""revocation_endpoint_auth_methods_supported":["none"]}"#
        )
    );
}

#[test]
fn token_exchange_adds_its_grant_type() {
    let supported = native_client_auth_contract(BASE, true).grant_types_supported;
    assert_eq!(
        supported,
        ["authorization_code", "refresh_token", TOKEN_EXCHANGE]
    );
}
