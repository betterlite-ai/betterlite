#![cfg(test)]

use std::time::Duration;

use rstest::rstest;

use crate::api::skills::hook::SkillsInjectionHook;
use crate::api::skills::hook::settings::{
    DEFAULT_MAX_ITERATIONS, DEFAULT_SANDBOX_TIMEOUT, HookKwargs, HookSettings,
    LITELLM_SKILL_ID_PREFIX,
};

#[test]
fn hook_init_defaults_are_ten_iterations_and_120_seconds() {
    assert_eq!(DEFAULT_MAX_ITERATIONS, 10);
    assert_eq!(DEFAULT_SANDBOX_TIMEOUT, Duration::from_secs(120));
    let settings = HookSettings::from(HookKwargs::default());
    assert_eq!(settings.max_iterations, 10);
    assert_eq!(settings.sandbox_timeout, Duration::from_secs(120));
}

#[test]
fn hook_init_skill_prefix_is_the_wire_value() {
    assert_eq!(LITELLM_SKILL_ID_PREFIX, "litellm_skill_");
}

#[rstest]
#[case::both(
    HookKwargs { max_iterations: Some(3), sandbox_timeout: Some(Duration::from_secs(7)) },
    3,
    Duration::from_secs(7)
)]
#[case::iterations_only(
    HookKwargs { max_iterations: Some(1), sandbox_timeout: None },
    1,
    Duration::from_secs(120)
)]
#[case::timeout_only(
    HookKwargs { max_iterations: None, sandbox_timeout: Some(Duration::from_secs(5)) },
    10,
    Duration::from_secs(5)
)]
fn hook_init_kwargs_replace_the_defaults(
    #[case] kwargs: HookKwargs,
    #[case] max_iterations: u32,
    #[case] sandbox_timeout: Duration,
) {
    let settings = HookSettings::from(kwargs);
    assert_eq!(settings.max_iterations, max_iterations);
    assert_eq!(settings.sandbox_timeout, sandbox_timeout);
}

#[test]
fn hook_init_stores_the_settings_on_the_hook() {
    let hook = SkillsInjectionHook {
        prompt_handler: (),
        executor: (),
        settings: HookSettings::from(HookKwargs {
            max_iterations: Some(4),
            sandbox_timeout: None,
        }),
    };
    assert_eq!(hook.settings.max_iterations, 4);
    assert_eq!(hook.settings.sandbox_timeout, Duration::from_secs(120));
}
