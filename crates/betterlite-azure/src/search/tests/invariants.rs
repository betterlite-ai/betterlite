use std::sync::atomic::Ordering;

use betterlite_config::loader::EnvironmentLookup as _;
use betterlite_config::sections::azure_default_chain::AzureCredentialType;
use betterlite_config::sections::azure_search::{
    CONNECTION_ID_ENV, MODEL_ENV, PROJECT_ENDPOINT_ENV, Settings, TOKEN_ENV,
};
use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_default_chain::environment::ChainEnvironment;
use crate::azure_default_chain::error::DefaultChainError;
use crate::azure_default_chain::grant::{
    CertificateRecord, ChainToken, ClientSecretRecord, Grant, GrantError, Grants, LinkError,
    ManagedIdentityRecord, UsernamePasswordRecord, WorkloadRecord,
};
use crate::azure_default_chain::provider::{AzureTokenProvider, ProviderInner};
use crate::search::environment::{ProbeEnvironment, SettingsEnvironment};
use crate::search::error::AzureSearchError;
use crate::search::minter::{
    EntraTokenMinter as _, MintError, NativeMinter, default_entra_token_minter,
};
use crate::search::{AzureSearchConfig, ENTRA_SCOPE, UI_FRIENDLY_NAME};

use super::support::{ForbiddenMinter, minter, model_settings, token_settings};

#[test]
fn azure_search_credential_identity_isolation() {
    env_tokens_stay_with_their_settings();
    api_keys_stay_with_their_config();
    minters_stay_with_their_config();
    untrusted_bases_never_mint();
}

fn env_tokens_stay_with_their_settings() {
    let first_settings = token_settings("fixture-identity-a");
    let second_settings = token_settings("fixture-identity-b");
    let first = AzureSearchConfig {
        settings: &first_settings,
        minter: None,
    };
    let second = AzureSearchConfig {
        settings: &second_settings,
        minter: None,
    };
    let first_headers = first
        .validate_environment(Headers::new(), None, None)
        .unwrap();
    let second_headers = second
        .validate_environment(Headers::new(), None, None)
        .unwrap();
    assert_eq!(
        first_headers.get("Authorization").map(String::as_str),
        Some("Bearer fixture-identity-a")
    );
    assert_eq!(
        second_headers.get("Authorization").map(String::as_str),
        Some("Bearer fixture-identity-b")
    );
    assert_ne!(first_headers, second_headers);
    let repeated = first
        .validate_environment(Headers::new(), None, None)
        .unwrap();
    assert_eq!(repeated, first_headers);
}

fn api_keys_stay_with_their_config() {
    let settings = token_settings("fixture-identity-a");
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let key = SecretString::from("fixture-api-key");
    let keyed = config
        .validate_environment(Headers::new(), Some(&key), None)
        .unwrap();
    assert_eq!(
        keyed.get("api-key").map(String::as_str),
        Some("fixture-api-key")
    );
    assert_eq!(keyed.get("Authorization"), None);
}

fn minters_stay_with_their_config() {
    let first_minter = minter(Ok("fixture-minted-a"));
    let second_minter = minter(Ok("fixture-minted-b"));
    let settings = model_settings();
    let first = AzureSearchConfig {
        settings: &settings,
        minter: Some(&first_minter),
    };
    let second = AzureSearchConfig {
        settings: &settings,
        minter: Some(&second_minter),
    };
    let first_headers = first
        .validate_environment(Headers::new(), None, None)
        .unwrap();
    let second_headers = second
        .validate_environment(Headers::new(), None, None)
        .unwrap();
    assert_eq!(
        first_headers.get("Authorization").map(String::as_str),
        Some("Bearer fixture-minted-a")
    );
    assert_eq!(
        second_headers.get("Authorization").map(String::as_str),
        Some("Bearer fixture-minted-b")
    );
    assert_eq!(first_minter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_minter.calls.load(Ordering::SeqCst), 1);
}

fn untrusted_bases_never_mint() {
    let settings = model_settings();
    let forbidden = ForbiddenMinter;
    let config = AzureSearchConfig {
        settings: &settings,
        minter: Some(&forbidden),
    };
    let refused = config
        .validate_environment(Headers::new(), None, Some("https://evil.example"))
        .unwrap_err();
    assert!(matches!(refused, AzureSearchError::Credential(_)));
    let fixture = minter(Ok("fixture-unused"));
    let guarded = AzureSearchConfig {
        settings: &settings,
        minter: Some(&fixture),
    };
    drop(
        guarded
            .validate_environment(Headers::new(), None, Some("https://evil.example"))
            .unwrap_err(),
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn azure_search_secret_safe_failure() {
    settings_and_config_debug_redact();
    credential_failures_do_not_echo_secrets();
    mint_failures_do_not_echo_secrets();
}

fn settings_and_config_debug_redact() {
    let settings = token_settings("fixture-env-secret");
    assert!(!format!("{settings:?}").contains("fixture-env-secret"));
    let fixture = minter(Ok("fixture-minted-secret"));
    let config = AzureSearchConfig {
        settings: &settings,
        minter: Some(&fixture),
    };
    let debug = format!("{config:?}");
    assert!(!debug.contains("fixture-env-secret"));
    assert!(!debug.contains("fixture-minted-secret"));
    assert!(debug.contains("has_minter: true"));
}

fn credential_failures_do_not_echo_secrets() {
    let settings = token_settings("fixture-env-secret");
    let fixture = minter(Ok("fixture-minted-secret"));
    let config = AzureSearchConfig {
        settings: &settings,
        minter: Some(&fixture),
    };
    let error = config
        .validate_environment(Headers::new(), None, Some("https://evil.example"))
        .unwrap_err();
    assert!(matches!(error, AzureSearchError::Credential(_)));
    assert!(!error.to_string().contains("fixture-env-secret"));
    assert!(!error.to_string().contains("fixture-minted-secret"));
    assert!(!format!("{error:?}").contains("fixture-env-secret"));
    assert!(!format!("{error:?}").contains("fixture-minted-secret"));
}

fn mint_failures_do_not_echo_secrets() {
    let settings = model_settings();
    let failing = minter(Err("fixture mint failure"));
    let config = AzureSearchConfig {
        settings: &settings,
        minter: Some(&failing),
    };
    let error = config
        .validate_environment(Headers::new(), None, None)
        .unwrap_err();
    assert!(!error.to_string().contains("fixture-minted-secret"));
    assert!(!format!("{error:?}").contains("fixture-minted-secret"));
    let empty_key = SecretString::from("");
    let succeeding = minter(Ok("fixture-second-mint"));
    let with_empty_key = AzureSearchConfig {
        settings: &settings,
        minter: Some(&succeeding),
    };
    let merged = with_empty_key
        .validate_environment(Headers::new(), Some(&empty_key), None)
        .unwrap();
    assert_eq!(
        merged.get("Authorization").map(String::as_str),
        Some("Bearer fixture-second-mint")
    );
}

#[test]
fn azure_search_settings_are_the_environment() {
    let settings = Settings {
        project_endpoint: Some(String::from("https://project.example")),
        model: Some(String::from("gpt-4.1")),
        connection_id: Some(String::from("connection-1")),
        token: Some(SecretString::from("fixture-env-token")),
    };
    let environment = SettingsEnvironment {
        settings: &settings,
    };
    assert_eq!(
        environment.lookup(PROJECT_ENDPOINT_ENV).as_deref(),
        Some("https://project.example")
    );
    assert_eq!(environment.lookup(MODEL_ENV).as_deref(), Some("gpt-4.1"));
    assert_eq!(
        environment.lookup(CONNECTION_ID_ENV).as_deref(),
        Some("connection-1")
    );
    assert_eq!(
        environment.lookup(TOKEN_ENV).as_deref(),
        Some("fixture-env-token")
    );
    assert_eq!(environment.lookup("UNRELATED").as_deref(), None);
    let probe = ProbeEnvironment {
        settings: &settings,
    };
    assert_eq!(
        probe.lookup(TOKEN_ENV).as_deref(),
        Some("pending-minted-token")
    );
    assert_eq!(
        probe.lookup(PROJECT_ENDPOINT_ENV).as_deref(),
        Some("https://project.example")
    );
    assert_eq!(probe.lookup("UNRELATED").as_deref(), None);
}

#[test]
fn azure_search_native_minter_mints() {
    let minter = native_minter(Ok("fixture-native-token"));
    assert_eq!(
        minter.mint().unwrap().expose_secret(),
        "fixture-native-token"
    );
    let failing = native_minter(Err("fixture native denial"));
    let error = failing.mint().unwrap_err();
    assert!(matches!(
        error,
        MintError::Chain(DefaultChainError::Credential {
            unavailable: false,
            message,
        }) if message == "fixture native denial"
    ));
}

#[test]
fn azure_search_default_minter_uses_the_entra_scope() {
    let environment = ChainEnvironment {
        scope: None,
        credential: Some(AzureCredentialType::ManagedIdentityCredential),
        client_id: Some(String::from("fixture-client-id")),
        tenant_id: None,
        client_secret: None,
        username: None,
        password: None,
        federated_token_file: None,
        certificate_path: None,
        certificate_password: None,
        client_certificate_path: None,
        client_certificate_password: None,
        client_send_certificate_chain: false,
        token_credentials: String::new(),
        authority_host: None,
    };
    let grants = StubGrants {
        outcome: Ok("fixture-native-token"),
    };
    let minter = default_entra_token_minter(&environment, &grants).unwrap();
    assert_eq!(minter.provider.scope, ENTRA_SCOPE);
    assert_eq!(
        minter.mint().unwrap().expose_secret(),
        "fixture-native-token"
    );
}

#[test]
fn azure_search_ui_friendly_name() {
    assert_eq!(UI_FRIENDLY_NAME, "Grounding with Bing Search");
}

fn native_minter(outcome: Result<&'static str, &'static str>) -> NativeMinter<'static, StubGrants> {
    NativeMinter {
        provider: AzureTokenProvider {
            credential: AzureCredentialType::ManagedIdentityCredential,
            scope: String::from(ENTRA_SCOPE),
            warnings: Vec::new(),
            inner: ProviderInner::Single(StubGrant { outcome }),
        },
    }
}

struct StubGrants {
    outcome: Result<&'static str, &'static str>,
}

struct StubGrant {
    outcome: Result<&'static str, &'static str>,
}

impl Grant for StubGrant {
    fn token(&self) -> Result<ChainToken, LinkError> {
        match self.outcome {
            Ok(token) => Ok(ChainToken {
                token: SecretString::from(token),
                expires_on: None,
                token_type: None,
                refresh_on: None,
            }),
            Err(message) => Err(LinkError::Authentication(message.to_owned())),
        }
    }
}

impl StubGrants {
    const fn grant(&self) -> StubGrant {
        StubGrant {
            outcome: self.outcome,
        }
    }
}

impl Grants for StubGrants {
    type Grant<'grants>
        = StubGrant
    where
        Self: 'grants;

    fn client_secret<'grants>(
        &'grants self,
        _record: &ClientSecretRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        Ok(self.grant())
    }

    fn certificate<'grants>(
        &'grants self,
        _record: &CertificateRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        Ok(self.grant())
    }

    fn username_password<'grants>(
        &'grants self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        Ok(self.grant())
    }

    fn workload<'grants>(
        &'grants self,
        _record: &WorkloadRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        Ok(self.grant())
    }

    fn managed_identity<'grants>(
        &'grants self,
        _record: &ManagedIdentityRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        Ok(self.grant())
    }
}
