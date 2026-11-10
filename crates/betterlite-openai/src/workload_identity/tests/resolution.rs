use betterlite_config::sections::openai_credentials::Settings as CredentialsSettings;
use betterlite_config::sections::openai_workload_identity::Settings as IdentitySettings;

use super::support::secret;
use crate::workload_identity::config::{
    OpenAiWorkloadIdentityConfig, WorkloadIdentityEnvironment, resolve_config,
};
use crate::workload_identity::error::WorkloadIdentityError;

fn full_identity() -> IdentitySettings {
    IdentitySettings {
        identity_provider_id: Some("idp".to_owned()),
        service_account_id: Some("sa".to_owned()),
        token_file: Some("/run/token".to_owned()),
    }
}

fn expected() -> OpenAiWorkloadIdentityConfig {
    OpenAiWorkloadIdentityConfig {
        identity_provider_id: "idp".to_owned(),
        service_account_id: "sa".to_owned(),
        token_file: "/run/token".to_owned(),
    }
}

struct Inputs<'text> {
    api_key: Option<&'text str>,
    env_key: Option<&'text str>,
    api_base: Option<&'text str>,
    litellm_base: Option<&'text str>,
    env_base: Option<&'text str>,
}

const NONE: Inputs<'static> = Inputs {
    api_key: None,
    env_key: None,
    api_base: None,
    litellm_base: None,
    env_base: None,
};

fn resolve(
    inputs: &Inputs<'_>,
    identity: &IdentitySettings,
) -> Result<Option<OpenAiWorkloadIdentityConfig>, WorkloadIdentityError> {
    let credentials = inputs
        .env_base
        .map_or_else(CredentialsSettings::default, |base| CredentialsSettings {
            api_base: base.to_owned(),
            organization: None,
        });
    let api_key = inputs.api_key.map(secret);
    let env_key = inputs.env_key.map(secret);
    resolve_config(
        api_key.as_ref(),
        inputs.api_base,
        WorkloadIdentityEnvironment {
            env_api_key: env_key.as_ref(),
            litellm_api_base: inputs.litellm_base,
            credentials: &credentials,
            identity,
        },
    )
}

fn resolved(inputs: &Inputs<'_>) -> Option<OpenAiWorkloadIdentityConfig> {
    resolve(inputs, &full_identity()).unwrap()
}

pub(super) fn static_key_disables_wif() {
    assert_eq!(resolved(&NONE), Some(expected()));
    let explicit = Inputs {
        api_key: Some("sk-explicit"),
        ..NONE
    };
    assert_eq!(resolved(&explicit), None);
    let env_only = Inputs {
        env_key: Some("sk-env"),
        ..NONE
    };
    assert_eq!(resolved(&env_only), None);
    let blank_then_env = Inputs {
        api_key: Some(" \t\n"),
        env_key: Some("sk-env"),
        ..NONE
    };
    assert_eq!(resolved(&blank_then_env), None);
    let blank_explicit = Inputs {
        api_key: Some(" \u{1c}\u{1f} "),
        ..NONE
    };
    assert_eq!(resolved(&blank_explicit), Some(expected()));
    let blank_env = Inputs {
        env_key: Some("\n"),
        ..NONE
    };
    assert_eq!(resolved(&blank_env), Some(expected()));
    let empty_both = Inputs {
        api_key: Some(""),
        env_key: Some(""),
        ..NONE
    };
    assert_eq!(resolved(&empty_both), Some(expected()));
}

pub(super) fn three_fields_required() {
    let missing = [
        IdentitySettings {
            identity_provider_id: None,
            ..full_identity()
        },
        IdentitySettings {
            service_account_id: None,
            ..full_identity()
        },
        IdentitySettings {
            token_file: None,
            ..full_identity()
        },
        IdentitySettings::default(),
    ];
    for identity in &missing {
        assert_eq!(resolve(&NONE, identity).unwrap(), None);
    }
    assert_eq!(resolve(&NONE, &full_identity()).unwrap(), Some(expected()));
}

fn with_base(base: &str) -> Inputs<'_> {
    Inputs {
        api_base: Some(base),
        ..NONE
    }
}

pub(super) fn https_openai_host_only() {
    scheme_and_host();
    base_precedence();
    foreign_bases();
    unparsable_base();
}

fn scheme_and_host() {
    let allowed = [
        "https://api.openai.com/v1",
        "https://eu.api.openai.com/v1",
        "HTTPS://API.OPENAI.COM/v1",
        " \thttps://api.openai.com/v1",
        "ht\ntps://api.openai.com/v1",
    ];
    for base in allowed {
        assert_eq!(resolved(&with_base(base)), Some(expected()), "{base}");
    }
    let refused = [
        "http://api.openai.com/v1",
        "ftp://api.openai.com/v1",
        "https://api.openai.com.attacker.example/v1",
        "https://fakeapi.openai.com",
        "https://example.com/v1",
        "api.openai.com/v1",
        "https://",
        "mailto:ops@example.test",
    ];
    for base in refused {
        assert_eq!(resolved(&with_base(base)), None, "{base}");
    }
}

const OPENAI: &str = "https://api.openai.com";
const OTHER: &str = "https://y.example";

fn base_precedence() {
    let chosen = [
        Inputs {
            api_base: Some(OPENAI),
            litellm_base: Some("https://x.example"),
            env_base: Some(OTHER),
            ..NONE
        },
        Inputs {
            api_base: Some(""),
            litellm_base: Some(OPENAI),
            env_base: Some(OTHER),
            ..NONE
        },
        Inputs {
            litellm_base: Some(OPENAI),
            env_base: Some(OTHER),
            ..NONE
        },
        Inputs {
            env_base: Some("https://us.api.openai.com"),
            ..NONE
        },
    ];
    for inputs in &chosen {
        assert_eq!(resolved(inputs), Some(expected()));
    }
}

fn foreign_bases() {
    let foreign = [
        Inputs {
            api_base: Some("https://x.example"),
            litellm_base: Some(OPENAI),
            ..NONE
        },
        Inputs {
            litellm_base: Some("http://api.openai.com"),
            ..NONE
        },
        Inputs {
            env_base: Some("https://gateway.example/v1"),
            ..NONE
        },
    ];
    for inputs in &foreign {
        assert_eq!(resolved(inputs), None);
    }
}

fn unparsable_base() {
    for broken in [
        "http://[::1",
        "https://[::1",
        "https://api.openai.com\u{2100}/v1",
    ] {
        let outcome = resolve(&with_base(broken), &full_identity());
        assert!(
            matches!(outcome, Err(WorkloadIdentityError::ApiBase(_))),
            "{broken}"
        );
    }
}
