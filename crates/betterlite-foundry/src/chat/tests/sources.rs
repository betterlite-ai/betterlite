use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::CredentialSources as _;
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use secrecy::SecretString;

use crate::chat::sources::NoSources;

#[test]
fn no_sources_refuse_grant_requests() {
    let secret = ClientSecretRecord {
        tenant_id: String::from("tenant"),
        client_id: String::from("client"),
        client_secret: SecretString::from("secret"),
        scope: String::from("scope"),
    };
    let password = UsernamePasswordRecord {
        client_id: String::from("client"),
        username: String::from("user"),
        password: SecretString::from("password"),
        tenant_id: None,
        scope: String::from("scope"),
    };
    let refused = [
        NoSources
            .client_secret(&secret)
            .err()
            .map(|failure| failure.message),
        NoSources
            .username_password(&password)
            .err()
            .map(|failure| failure.message),
    ];
    assert_eq!(
        refused,
        [
            Some(String::from("no client secret source is configured")),
            Some(String::from(
                "no username and password source is configured"
            )),
        ]
    );
}

#[test]
fn no_sources_refuse_refresh_and_exchange_requests() {
    let oidc = OidcExchangeInput {
        azure_ad_token: SecretString::from("token"),
        azure_client_id: None,
        azure_tenant_id: None,
        scope: None,
    };
    let refused = [
        NoSources
            .sdk_refresh("scope")
            .err()
            .map(|failure| failure.message),
        NoSources.oidc(&oidc).err().map(|failure| failure.message),
    ];
    assert_eq!(
        refused,
        [
            Some(String::from("no SDK refresh source is configured")),
            Some(String::from("no OIDC source is configured")),
        ]
    );
    assert!(NoSources.default_credential("scope").is_none());
}
