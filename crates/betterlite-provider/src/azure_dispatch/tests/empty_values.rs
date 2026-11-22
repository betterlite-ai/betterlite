use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_dispatch::call::{Credentials, Operation};
use crate::azure_dispatch::resolve::resolve;

use super::cases::{AZURE_BASE, base_only};
use super::support;

// An empty global value, secret, environment version, body token or api type counts as unset, so the next rank applies.
#[test]
fn azure_empty_values_rank_as_unset() {
    let mut empty_global = support::composition(&[("AZURE_API_BASE", "env-base")]);
    empty_global.globals.api_base = Some(String::new());
    let from_global = resolve(
        &empty_global,
        support::credentials(),
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the empty global base");
    assert_eq!(from_global.api_base.as_deref(), Some("env-base"));

    let mut empty_key = base_only();
    empty_key.api_key = Some(support::key(""));
    let keyed = resolve(
        &support::composition(&[("AZURE_OPENAI_API_KEY", "env-openai-key")]),
        empty_key,
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the empty explicit key");
    assert_eq!(
        keyed.api_key.as_ref().map(SecretString::expose_secret),
        Some("env-openai-key")
    );

    let empty_version = support::composition(&[("AZURE_API_VERSION", "")]);
    let defaulted = resolve(&empty_version, base_only(), Operation::Chat, Some("gpt-4o"))
        .expect("the empty environment version");
    assert_eq!(defaulted.api_version.as_deref(), Some("2025-02-01-preview"));

    let empty_token = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        extra_body: Some(support::json(r#"{"azure_ad_token":""}"#)),
        ..support::credentials()
    };
    let from_token_env = resolve(
        &support::composition(&[("AZURE_AD_TOKEN", "env-token")]),
        empty_token,
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the empty body token");
    assert_eq!(
        from_token_env
            .azure_ad_token
            .as_ref()
            .map(SecretString::expose_secret),
        Some("env-token")
    );

    let empty_type = support::composition(&[("AZURE_API_TYPE", "")]);
    let typed = resolve(&empty_type, base_only(), Operation::Chat, Some("gpt-4o"))
        .expect("the empty api type");
    assert_eq!(typed.api_type, "azure");
}
