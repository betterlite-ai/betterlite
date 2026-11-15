#![cfg(test)]

use std::time::Duration;

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::vigil_runtime::Settings as EnvironmentDefaults;

use crate::generic_guardrail_contracts::UnreachableFallback;
use crate::third_party_init_b::vigil_guard::{VigilGuardArgs, initialize_vigil_guard};
use crate::vigil_guard::DEFAULT_TIMEOUT;
use crate::vigil_guard::handler_protocol::RequestTimeout;
use crate::vigil_runtime::SUPPORTED_EVENT_HOOKS;

use super::cases::{BuildCase, BuildExpectation, BuiltSettings};
use super::fixtures::{args, env_defaults, loaded_defaults};

const THREE_HOOKS: [GuardrailEventHooks; 3] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::DuringCall,
];

fn args_from_json(body: &str, name: Option<&str>) -> VigilGuardArgs {
    let params = decode_litellm_params(body.as_bytes()).expect("the guardrail params decode");
    initialize_vigil_guard(&params, name).expect("the vigil arguments build")
}

fn explicit_args(fallback: &str, timeout: Option<f64>) -> VigilGuardArgs {
    args(
        Some("https://vigil.example"),
        Some("sk-vigil"),
        fallback,
        timeout,
    )
}

fn args_with_hook(event_hook: &str) -> VigilGuardArgs {
    let mut arguments = explicit_args("fail_closed", None);
    arguments.event_hook = LitellmParamsMode::One(event_hook.to_owned());
    arguments
}

fn built(settings: BuiltSettings) -> BuildExpectation {
    BuildExpectation::Built(Box::new(settings))
}

fn failure(fragment: &'static str) -> BuildExpectation {
    BuildExpectation::Error(fragment)
}

fn standard_settings() -> BuiltSettings {
    BuiltSettings {
        api_base: "https://vigil.example",
        api_key: "sk-vigil",
        timeout: DEFAULT_TIMEOUT,
        fallback: UnreachableFallback::FailClosed,
        hooks: SUPPORTED_EVENT_HOOKS.to_vec(),
    }
}

fn env_settings() -> BuiltSettings {
    BuiltSettings {
        api_base: "https://env.vigil",
        api_key: "sk-env",
        ..standard_settings()
    }
}

fn seconds(total: u64, connect: u64) -> RequestTimeout {
    RequestTimeout {
        total: Duration::from_secs(total),
        connect: Duration::from_secs(connect),
    }
}

fn millis(total: u64, connect: u64) -> RequestTimeout {
    RequestTimeout {
        total: Duration::from_millis(total),
        connect: Duration::from_millis(connect),
    }
}

fn loaded_env_defaults() -> EnvironmentDefaults {
    loaded_defaults(
        "general_settings: {}\n",
        &[
            ("VIGIL_GUARD_URL", "https://env.vigil/"),
            ("VIGIL_GUARD_API_KEY", "sk-env"),
        ],
    )
}

fn standard() -> BuildCase {
    BuildCase::new(
        explicit_args("fail_closed", None),
        EnvironmentDefaults::default(),
        built(standard_settings()),
    )
}

fn env_case(args: VigilGuardArgs, defaults: EnvironmentDefaults) -> BuildCase {
    BuildCase {
        args,
        defaults,
        expected: built(env_settings()),
        ..standard()
    }
}

fn fallback_case(value: &str) -> BuildCase {
    BuildCase {
        args: explicit_args(value, None),
        ..standard()
    }
}

pub(crate) fn environment_defaults_fill_missing_arguments() -> BuildCase {
    let arguments = args(None, None, "fail_closed", None);
    env_case(arguments, loaded_env_defaults())
}

pub(crate) fn explicit_arguments_override_environment_defaults() -> BuildCase {
    let arguments = args_from_json(
        r#"{"guardrail": "vigil_guard", "mode": "post_call", "api_base": "https://explicit.vigil///", "api_key": "sk-explicit", "unreachable_fallback": "fail_open", "timeout": 2.5, "default_on": true}"#,
        Some("vigil-check"),
    );
    BuildCase {
        args: arguments,
        defaults: loaded_env_defaults(),
        expected: built(BuiltSettings {
            api_base: "https://explicit.vigil",
            api_key: "sk-explicit",
            timeout: millis(2500, 2500),
            fallback: UnreachableFallback::FailOpen,
            ..standard_settings()
        }),
        ..standard()
    }
}

pub(crate) fn explicit_empty_credentials_fall_back_to_defaults() -> BuildCase {
    let arguments = args(Some(""), Some(""), "fail_closed", None);
    env_case(arguments, env_defaults("https://env.vigil/", "sk-env"))
}

pub(crate) fn a_long_timeout_caps_the_connect_bound() -> BuildCase {
    BuildCase {
        args: explicit_args("fail_closed", Some(20.0)),
        expected: built(BuiltSettings {
            timeout: seconds(20, 5),
            ..standard_settings()
        }),
        ..standard()
    }
}

pub(crate) fn an_invalid_timeout_is_rejected() -> BuildCase {
    BuildCase {
        args: explicit_args("fail_closed", Some(-1.0)),
        expected: failure("timeout"),
        ..standard()
    }
}

pub(crate) fn missing_api_base_names_the_field() -> BuildCase {
    BuildCase {
        args: args(None, Some("sk-guard"), "fail_closed", None),
        expected: failure("api_base"),
        ..standard()
    }
}

pub(crate) fn missing_api_key_names_the_field() -> BuildCase {
    BuildCase {
        args: args(Some("https://vigil.example"), None, "fail_closed", None),
        expected: failure("api_key"),
        ..standard()
    }
}

pub(crate) fn explicit_supported_event_hooks_take_precedence() -> BuildCase {
    BuildCase {
        args: args_with_hook("during_call"),
        hooks: Some(THREE_HOOKS.to_vec()),
        expected: built(BuiltSettings {
            hooks: THREE_HOOKS.to_vec(),
            ..standard_settings()
        }),
        ..standard()
    }
}

pub(crate) fn logging_only_is_accepted_for_the_apply_interface() -> BuildCase {
    BuildCase {
        args: args_with_hook("logging_only"),
        ..standard()
    }
}

pub(crate) fn an_unsupported_event_hook_is_rejected_in_strict_mode() -> BuildCase {
    BuildCase {
        args: args_with_hook("during_call"),
        expected: failure("during_call"),
        ..standard()
    }
}

pub(crate) fn a_relaxed_strict_mode_allows_the_hook() -> BuildCase {
    BuildCase {
        args: args_with_hook("during_call"),
        strict_modes: false,
        ..standard()
    }
}

pub(crate) fn the_fallback_value_is_matched_case_insensitively() -> BuildCase {
    let mut case = fallback_case("FAIL_OPEN");
    case.expected = built(BuiltSettings {
        fallback: UnreachableFallback::FailOpen,
        ..standard_settings()
    });
    case
}

pub(crate) fn another_fallback_value_fails_closed() -> BuildCase {
    fallback_case("open")
}
