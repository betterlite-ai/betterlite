#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use betterlite_models::sandbox_dispatch::{SandboxToolConfig, SandboxToolEntry};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::{ExposeSecret as _, SecretString};

use super::{SandboxRegistryError, SandboxToolRegistry};

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "no network in tests".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn configured(
    name: Option<&str>,
    provider: Option<&str>,
    api_key: Option<&str>,
    api_base: Option<&str>,
) -> SandboxToolEntry {
    SandboxToolEntry::Configured(SandboxToolConfig {
        name: name.map(str::to_owned),
        provider: provider.map(str::to_owned),
        api_key: api_key.map(str::to_owned),
        api_base: api_base.map(str::to_owned),
    })
}

fn register(
    entries: &[SandboxToolEntry],
    environment: &BTreeMap<String, String>,
) -> SandboxToolRegistry {
    let mut registry = SandboxToolRegistry::default();
    registry.register(entries, &secrets(), environment).unwrap();
    registry
}

fn text(value: &Option<SecretString>) -> Option<String> {
    value
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned())
}

#[test]
fn resolves_a_configured_tool_by_name() {
    let registry = register(
        &[configured(
            Some("tools"),
            Some("e2b"),
            Some("plain-key"),
            Some("https://e2b.test"),
        )],
        &BTreeMap::new(),
    );
    let tool = registry.resolve("tools").expect("the tool registers");
    assert_eq!(tool.provider, "e2b");
    assert_eq!(text(&tool.api_key).as_deref(), Some("plain-key"));
    assert_eq!(tool.api_base.as_deref(), Some("https://e2b.test"));
}

#[test]
fn an_unknown_name_resolves_to_none() {
    let registry = register(&[], &BTreeMap::new());
    assert!(registry.resolve("missing").is_none());
}

#[test]
fn resolves_os_environ_references_through_the_lookup() {
    let registry = register(
        &[configured(
            Some("tools"),
            Some("e2b"),
            Some("os.environ/E2B_API_KEY"),
            Some("os.environ/E2B_API_BASE"),
        )],
        &environment(&[
            ("E2B_API_KEY", "resolved-key"),
            ("E2B_API_BASE", "https://e2b.test"),
        ]),
    );
    let tool = registry.resolve("tools").expect("the tool registers");
    assert_eq!(text(&tool.api_key).as_deref(), Some("resolved-key"));
    assert_eq!(tool.api_base.as_deref(), Some("https://e2b.test"));
}

#[test]
fn a_missing_environment_variable_resolves_to_none() {
    let registry = register(
        &[configured(
            Some("tools"),
            Some("e2b"),
            Some("os.environ/MISSING"),
            None,
        )],
        &BTreeMap::new(),
    );
    let tool = registry.resolve("tools").expect("the tool registers");
    assert!(tool.api_key.is_none());
}

#[test]
fn an_environment_value_that_reads_as_a_flag_resolves_to_none() {
    let registry = register(
        &[configured(
            Some("tools"),
            Some("e2b"),
            Some("os.environ/FLAG"),
            None,
        )],
        &environment(&[("FLAG", "true")]),
    );
    let tool = registry.resolve("tools").expect("the tool registers");
    assert!(tool.api_key.is_none());
}

#[test]
fn an_empty_environment_value_resolves_to_empty_text() {
    let registry = register(
        &[configured(
            Some("tools"),
            Some("e2b"),
            Some("os.environ/EMPTY"),
            None,
        )],
        &environment(&[("EMPTY", "")]),
    );
    let tool = registry.resolve("tools").expect("the tool registers");
    assert_eq!(text(&tool.api_key).as_deref(), Some(""));
}

#[test]
fn a_value_without_the_os_environ_prefix_stays_as_written() {
    let registry = register(
        &[configured(
            Some("tools"),
            Some("e2b"),
            Some("aws_secret_manager/key"),
            Some("aws_secret_manager/base"),
        )],
        &BTreeMap::new(),
    );
    let tool = registry.resolve("tools").expect("the tool registers");
    assert_eq!(
        text(&tool.api_key).as_deref(),
        Some("aws_secret_manager/key")
    );
    assert_eq!(tool.api_base.as_deref(), Some("aws_secret_manager/base"));
}

#[test]
fn a_secret_lookup_failure_names_the_reference() {
    let mut registry = SandboxToolRegistry::default();
    let error = registry
        .register(
            &[configured(
                Some("tools"),
                Some("e2b"),
                Some("os.environ/oidc/bogus/audience"),
                None,
            )],
            &secrets(),
            &BTreeMap::new(),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        SandboxRegistryError::Secret { ref reference, .. }
            if reference == "os.environ/oidc/bogus/audience"
    ));
}

#[test]
fn skips_an_entry_missing_a_name() {
    let registry = register(
        &[
            configured(None, Some("e2b"), Some("key"), None),
            configured(Some(""), Some("e2b"), Some("key"), None),
            configured(Some("kept"), Some("e2b"), Some("key"), None),
        ],
        &BTreeMap::new(),
    );
    assert!(registry.resolve("kept").is_some());
    assert!(registry.resolve("").is_none());
}

#[test]
fn skips_an_entry_missing_a_provider() {
    let registry = register(
        &[
            configured(Some("no-provider"), None, Some("key"), None),
            configured(Some("empty-provider"), Some(""), Some("key"), None),
            configured(Some("kept"), Some("e2b"), Some("key"), None),
        ],
        &BTreeMap::new(),
    );
    assert!(registry.resolve("no-provider").is_none());
    assert!(registry.resolve("empty-provider").is_none());
    assert!(registry.resolve("kept").is_some());
}

#[test]
fn skips_an_entry_that_is_not_an_object() {
    let registry = register(
        &[
            SandboxToolEntry::NotAnObject,
            configured(Some("kept"), Some("e2b"), None, None),
        ],
        &BTreeMap::new(),
    );
    assert!(registry.resolve("kept").is_some());
}

#[test]
fn replaces_the_previous_registry_on_every_register_call() {
    let mut registry = register(
        &[
            configured(Some("first"), Some("e2b"), None, None),
            configured(Some("second"), Some("e2b"), None, None),
        ],
        &BTreeMap::new(),
    );
    registry
        .register(
            &[configured(Some("second"), Some("opensandbox"), None, None)],
            &secrets(),
            &BTreeMap::new(),
        )
        .unwrap();
    assert!(registry.resolve("first").is_none());
    let second = registry.resolve("second").expect("the second tool stays");
    assert_eq!(second.provider, "opensandbox");
}

#[test]
fn keeps_the_last_entry_for_a_duplicate_name() {
    let registry = register(
        &[
            configured(Some("tools"), Some("e2b"), Some("first-key"), None),
            configured(Some("tools"), Some("opensandbox"), Some("second-key"), None),
        ],
        &BTreeMap::new(),
    );
    let tool = registry.resolve("tools").expect("the tool registers");
    assert_eq!(tool.provider, "opensandbox");
    assert_eq!(text(&tool.api_key).as_deref(), Some("second-key"));
}
