#![cfg(test)]

use std::time::Duration;

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::vigil_runtime::Settings as EnvironmentDefaults;

use crate::generic_guardrail_contracts::UnreachableFallback;
use crate::third_party_init_b::vigil_guard::{VigilGuardArgs, initialize_vigil_guard};
use crate::vigil_guard::DEFAULT_TIMEOUT;
use crate::vigil_guard::handler_protocol::RequestTimeout;
use crate::vigil_runtime::SUPPORTED_EVENT_HOOKS;

use super::cases::{BuildCase, BuildExpectation, BuiltSettings, Case};
use super::fixtures::{args, args_with_hook, env_defaults, loaded_defaults};

fn args_from_json(body: &str, name: Option<&str>) -> VigilGuardArgs {
    let params = decode_litellm_params(body.as_bytes()).expect("the guardrail params decode");
    initialize_vigil_guard(&params, name).expect("the vigil arguments build")
}

pub(crate) fn environment_defaults_fill_missing_arguments() -> Case {
    Case::Build(Box::new(BuildCase {
        args: args(None, None, "fail_closed", None),
        defaults: loaded_defaults(
            "general_settings: {}\n",
            &[
                ("VIGIL_GUARD_URL", "https://env.vigil/"),
                ("VIGIL_GUARD_API_KEY", "sk-env"),
            ],
        ),
        hooks: None,
        strict_modes: true,
        expected: BuildExpectation::Built(BuiltSettings {
            api_base: "https://env.vigil",
            api_key: "sk-env",
            timeout: DEFAULT_TIMEOUT,
            fallback: UnreachableFallback::FailClosed,
            hooks: SUPPORTED_EVENT_HOOKS.to_vec(),
        }),
    }))
}

pub(crate) fn explicit_arguments_override_environment_defaults() -> Case {
    let arguments = args_from_json(
        r#"{"guardrail": "vigil_guard", "mode": "post_call", "api_base": "https://explicit.vigil///", "api_key": "sk-explicit", "unreachable_fallback": "fail_open", "timeout": 2.5, "default_on": true}"#,
        Some("vigil-check"),
    );
    Case::Build(Box::new(BuildCase {
        args: arguments,
        defaults: loaded_defaults(
            "general_settings: {}\n",
            &[
                ("VIGIL_GUARD_URL", "https://env.vigil"),
                ("VIGIL_GUARD_API_KEY", "sk-env"),
            ],
        ),
        hooks: None,
        strict_modes: true,
        expected: BuildExpectation::Built(BuiltSettings {
            api_base: "https://explicit.vigil",
            api_key: "sk-explicit",
            timeout: RequestTimeout {
                total: Duration::from_millis(2500),
                connect: Duration::from_millis(2500),
            },
            fallback: UnreachableFallback::FailOpen,
            hooks: SUPPORTED_EVENT_HOOKS.to_vec(),
        }),
    }))
}

pub(crate) fn explicit_empty_credentials_fall_back_to_defaults() -> Case {
    Case::Build(Box::new(BuildCase {
        args: args(Some(""), Some(""), "fail_closed", None),
        defaults: env_defaults("https://env.vigil/", "sk-env"),
        hooks: None,
        strict_modes: true,
        expected: BuildExpectation::Built(BuiltSettings {
            api_base: "https://env.vigil",
            api_key: "sk-env",
            timeout: DEFAULT_TIMEOUT,
            fallback: UnreachableFallback::FailClosed,
            hooks: SUPPORTED_EVENT_HOOKS.to_vec(),
        }),
    }))
}

pub(crate) fn a_long_timeout_caps_the_connect_bound() -> Case {
    Case::Build(Box::new(BuildCase {
        args: args(
            Some("https://vigil.example"),
            Some("sk-vigil"),
            "fail_closed",
            Some(20.0),
        ),
        defaults: EnvironmentDefaults::default(),
        hooks: None,
        strict_modes: true,
        expected: BuildExpectation::Built(BuiltSettings {
            api_base: "https://vigil.example",
            api_key: "sk-vigil",
            timeout: RequestTimeout {
                total: Duration::from_secs(20),
                connect: Duration::from_secs(5),
            },
            fallback: UnreachableFallback::FailClosed,
            hooks: SUPPORTED_EVENT_HOOKS.to_vec(),
        }),
    }))
}

pub(crate) fn an_invalid_timeout_is_rejected() -> Case {
    Case::Build(Box::new(BuildCase {
        args: args(
            Some("https://vigil.example"),
            Some("sk-vigil"),
            "fail_closed",
            Some(-1.0),
        ),
        defaults: EnvironmentDefaults::default(),
        hooks: None,
        strict_modes: true,
        expected: BuildExpectation::Error("timeout"),
    }))
}

pub(crate) fn missing_api_base_names_the_field() -> Case {
    Case::Build(Box::new(BuildCase {
        args: args(None, Some("sk-guard"), "fail_closed", None),
        defaults: EnvironmentDefaults::default(),
        hooks: None,
        strict_modes: true,
        expected: BuildExpectation::Error("api_base"),
    }))
}

pub(crate) fn missing_api_key_names_the_field() -> Case {
    Case::Build(Box::new(BuildCase {
        args: args(Some("https://vigil.example"), None, "fail_closed", None),
        defaults: EnvironmentDefaults::default(),
        hooks: None,
        strict_modes: true,
        expected: BuildExpectation::Error("api_key"),
    }))
}

pub(crate) fn explicit_supported_event_hooks_take_precedence() -> Case {
    Case::Build(Box::new(BuildCase {
        args: args_with_hook("https://vigil.example", "sk-vigil", "during_call"),
        defaults: EnvironmentDefaults::default(),
        hooks: Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::DuringCall,
        ]),
        strict_modes: true,
        expected: BuildExpectation::Built(BuiltSettings {
            api_base: "https://vigil.example",
            api_key: "sk-vigil",
            timeout: DEFAULT_TIMEOUT,
            fallback: UnreachableFallback::FailClosed,
            hooks: vec![
                GuardrailEventHooks::PreCall,
                GuardrailEventHooks::PostCall,
                GuardrailEventHooks::DuringCall,
            ],
        }),
    }))
}

pub(crate) fn logging_only_is_accepted_for_the_apply_interface() -> Case {
    Case::Build(Box::new(BuildCase {
        args: args_with_hook("https://vigil.example", "sk-vigil", "logging_only"),
        defaults: EnvironmentDefaults::default(),
        hooks: None,
        strict_modes: true,
        expected: BuildExpectation::Built(BuiltSettings {
            api_base: "https://vigil.example",
            api_key: "sk-vigil",
            timeout: DEFAULT_TIMEOUT,
            fallback: UnreachableFallback::FailClosed,
            hooks: SUPPORTED_EVENT_HOOKS.to_vec(),
        }),
    }))
}

pub(crate) fn an_unsupported_event_hook_is_rejected_in_strict_mode() -> Case {
    Case::Build(Box::new(BuildCase {
        args: args_with_hook("https://vigil.example", "sk-vigil", "during_call"),
        defaults: EnvironmentDefaults::default(),
        hooks: None,
        strict_modes: true,
        expected: BuildExpectation::Error("during_call"),
    }))
}

pub(crate) fn a_relaxed_strict_mode_allows_the_hook() -> Case {
    Case::Build(Box::new(BuildCase {
        args: args_with_hook("https://vigil.example", "sk-vigil", "during_call"),
        defaults: EnvironmentDefaults::default(),
        hooks: None,
        strict_modes: false,
        expected: BuildExpectation::Built(BuiltSettings {
            api_base: "https://vigil.example",
            api_key: "sk-vigil",
            timeout: DEFAULT_TIMEOUT,
            fallback: UnreachableFallback::FailClosed,
            hooks: SUPPORTED_EVENT_HOOKS.to_vec(),
        }),
    }))
}

pub(crate) fn the_fallback_value_is_matched_case_insensitively() -> Case {
    Case::Build(Box::new(BuildCase {
        args: args(
            Some("https://vigil.example"),
            Some("sk-vigil"),
            "FAIL_OPEN",
            None,
        ),
        defaults: EnvironmentDefaults::default(),
        hooks: None,
        strict_modes: true,
        expected: BuildExpectation::Built(BuiltSettings {
            api_base: "https://vigil.example",
            api_key: "sk-vigil",
            timeout: DEFAULT_TIMEOUT,
            fallback: UnreachableFallback::FailOpen,
            hooks: SUPPORTED_EVENT_HOOKS.to_vec(),
        }),
    }))
}

pub(crate) fn another_fallback_value_fails_closed() -> Case {
    Case::Build(Box::new(BuildCase {
        args: args(
            Some("https://vigil.example"),
            Some("sk-vigil"),
            "open",
            None,
        ),
        defaults: EnvironmentDefaults::default(),
        hooks: None,
        strict_modes: true,
        expected: BuildExpectation::Built(BuiltSettings {
            api_base: "https://vigil.example",
            api_key: "sk-vigil",
            timeout: DEFAULT_TIMEOUT,
            fallback: UnreachableFallback::FailClosed,
            hooks: SUPPORTED_EVENT_HOOKS.to_vec(),
        }),
    }))
}
