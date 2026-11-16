use betterlite_models::identity::provider_id::ProviderId;
use secrecy::SecretString;

use super::support;
use crate::azure_urls::AzureUrlError;
use crate::responses::capabilities::PROVIDER;
use crate::responses::url::complete_url;

pub(super) fn responses_base_and_api_version() {
    let preview = support::settings("preview");
    let dated = support::settings("2024-10-21");
    let host = Some("https://res.openai.azure.com");
    // the preview default targets the v1 surface, a dated default the classic route
    assert_eq!(
        complete_url(host, None, &preview).unwrap(),
        "https://res.openai.azure.com/openai/v1/responses?api-version=preview"
    );
    assert_eq!(
        complete_url(host, None, &dated).unwrap(),
        "https://res.openai.azure.com/openai/responses?api-version=2024-10-21"
    );
    // an explicit version wins over the configured default
    assert_eq!(
        complete_url(host, Some("2025-01-01-preview"), &preview).unwrap(),
        "https://res.openai.azure.com/openai/responses?api-version=2025-01-01-preview"
    );
    // an api-version already in api_base wins over both
    assert_eq!(
        complete_url(
            Some("https://host/openai/responses?api-version=2024-06-01"),
            Some("2025-01-01-preview"),
            &preview
        )
        .unwrap(),
        "https://host/openai/responses?api-version=2024-06-01"
    );
    // an empty explicit version is unset, so the configured default fills it
    assert_eq!(
        complete_url(Some("https://host"), Some(""), &dated).unwrap(),
        "https://host/openai/responses?api-version=2024-10-21"
    );
    // a base path keeps its prefix
    assert_eq!(
        complete_url(Some("https://host/some/base/"), None, &dated).unwrap(),
        "https://host/some/base/openai/responses?api-version=2024-10-21"
    );
    assert!(matches!(
        complete_url(None, None, &preview),
        Err(AzureUrlError::MissingApiBase)
    ));
    assert_eq!(PROVIDER, ProviderId::Azure);
    environment_headers();
}

pub(super) fn environment_headers() {
    let key = SecretString::from("secret");
    // an api-key already on the headers is kept over the parameter
    let kept = support::environment_headers(&[("api-key", "header")], Some(&key)).unwrap();
    assert_eq!(kept["api-key"], "header");
    // the parameter adds the header when it is missing
    let added = support::environment_headers(&[], Some(&key)).unwrap();
    assert_eq!(added["api-key"], "secret");
    // an empty key counts as unset, so the token path runs and adds nothing here
    let empty = SecretString::from("");
    let token = support::environment_headers(&[], Some(&empty)).unwrap();
    assert!(!token.contains_key("api-key"));
    assert!(!token.contains_key("Authorization"));
}
