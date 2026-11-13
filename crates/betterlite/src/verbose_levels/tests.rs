#![cfg(test)]

use rstest::rstest;

use super::{EMPTY_PLAN, LevelPlan, LogLevel, plan_verbose_levels};

const ALL_INFO: LevelPlan = LevelPlan {
    package: Some(LogLevel::Info),
    router: Some(LogLevel::Info),
    proxy: Some(LogLevel::Info),
};

const ALL_DEBUG: LevelPlan = LevelPlan {
    package: Some(LogLevel::Debug),
    router: Some(LogLevel::Debug),
    proxy: Some(LogLevel::Debug),
};

const ROUTER_PROXY_INFO: LevelPlan = LevelPlan {
    package: None,
    router: Some(LogLevel::Info),
    proxy: Some(LogLevel::Info),
};

const ROUTER_PROXY_DEBUG: LevelPlan = LevelPlan {
    package: None,
    router: Some(LogLevel::Debug),
    proxy: Some(LogLevel::Debug),
};

fn plan_with(
    worker_config: Option<&str>,
    worker_config_is_file: bool,
    litellm_log: &str,
) -> (LevelPlan, Vec<String>) {
    let mut warnings = Vec::new();
    let plan = plan_verbose_levels(
        worker_config,
        worker_config_is_file,
        litellm_log,
        &mut |text| {
            warnings.push(text.to_owned());
        },
    );
    (plan, warnings)
}

#[test]
fn absent_worker_config_plans_nothing() {
    let (plan, warnings) = plan_with(None, false, "DEBUG");

    assert_eq!(plan, EMPTY_PLAN);
    assert!(warnings.is_empty());
}

#[test]
fn a_worker_config_that_names_a_file_plans_nothing() {
    let (plan, warnings) = plan_with(
        Some(r#"{"debug":true,"detailed_debug":true}"#),
        true,
        "DEBUG",
    );

    assert_eq!(plan, EMPTY_PLAN);
    assert!(warnings.is_empty());
}

#[test]
fn debug_alone_turns_every_logger_to_info() {
    let (plan, warnings) = plan_with(Some(r#"{"debug":true}"#), false, "DEBUG");

    assert_eq!(plan, ALL_INFO);
    assert!(warnings.is_empty());
}

#[test]
fn debug_ignores_the_environment_log_level() {
    let (plan, _warnings) = plan_with(
        Some(r#"{"debug":true,"detailed_debug":false}"#),
        false,
        "DEBUG",
    );

    assert_eq!(plan, ALL_INFO);
}

#[test]
fn detailed_debug_after_debug_turns_every_logger_to_debug() {
    let (plan, _warnings) = plan_with(Some(r#"{"debug":true,"detailed_debug":true}"#), false, "");

    assert_eq!(plan, ALL_DEBUG);
}

#[test]
fn detailed_debug_alone_turns_every_logger_to_debug() {
    let (plan, _warnings) = plan_with(Some(r#"{"detailed_debug":true}"#), false, "");

    assert_eq!(plan, ALL_DEBUG);
}

#[rstest]
#[case::upper_info("INFO", ROUTER_PROXY_INFO)]
#[case::lower_info("info", ROUTER_PROXY_INFO)]
#[case::upper_debug("DEBUG", ROUTER_PROXY_DEBUG)]
#[case::lower_debug("debug", ROUTER_PROXY_DEBUG)]
#[case::unset("", EMPTY_PLAN)]
#[case::other_level("WARN", EMPTY_PLAN)]
#[case::padded(" DEBUG ", EMPTY_PLAN)]
fn both_flags_false_reads_only_info_or_debug(
    #[case] litellm_log: &str,
    #[case] expected: LevelPlan,
) {
    let (plan, warnings) = plan_with(
        Some(r#"{"debug":false,"detailed_debug":false}"#),
        false,
        litellm_log,
    );

    assert_eq!(plan, expected);
    assert!(warnings.is_empty());
}

#[rstest]
#[case::debug_false_only(r#"{"debug":false}"#)]
#[case::detailed_debug_false_only(r#"{"detailed_debug":false}"#)]
#[case::null_flags(r#"{"debug":null,"detailed_debug":null}"#)]
#[case::empty_object("{}")]
fn an_absent_flag_never_reads_the_environment(#[case] worker_config: &str) {
    let (plan, _warnings) = plan_with(Some(worker_config), false, "DEBUG");

    assert_eq!(plan, EMPTY_PLAN);
}

#[test]
fn a_bad_worker_config_warns_once_and_plans_nothing() {
    let (plan, warnings) = plan_with(Some("{"), false, "DEBUG");

    assert_eq!(plan, EMPTY_PLAN);
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].starts_with("Failed to init verbose loggers: "),
        "{}",
        warnings[0]
    );
}

#[rstest]
#[case::empty_array("[]")]
#[case::true_pair("[true,true]")]
#[case::false_pair("[false,false]")]
fn a_worker_config_that_is_not_an_object_warns_and_plans_nothing(#[case] raw: &str) {
    let (plan, warnings) = plan_with(Some(raw), false, "DEBUG");

    assert_eq!(plan, EMPTY_PLAN);
    assert_eq!(warnings.len(), 1);
}
