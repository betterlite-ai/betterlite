use std::collections::BTreeMap;

use rstest::rstest;

use crate::cache_accounting::dialect::{
    gpt_version, model_map_breakpoint_flag, supports_openai_breakpoint, targets_openai_api,
    targets_openai_breakpoint,
};
use crate::cache_accounting::tests::support::{Facts, dialect, json};

#[rstest]
#[case::bare_major("gpt-5", Some((5, 0)))]
#[case::single_dot("gpt-4.5", Some((4, 5)))]
#[case::minimum("gpt-5.6", Some((5, 6)))]
#[case::two_digit_minor("gpt-5.10", Some((5, 10)))]
#[case::version_prefix_wins("gpt-5.6.1", Some((5, 6)))]
#[case::trailing_text("gpt-5-beta", Some((5, 0)))]
#[case::trailing_dot("gpt-5.", Some((5, 0)))]
#[case::dotted_minor_then_text("gpt-5.6beta", Some((5, 6)))]
#[case::provider_prefix("openai/gpt-5.6", Some((5, 6)))]
#[case::upper_case("GPT-5.6", Some((5, 6)))]
#[case::not_at_start("xgpt-5.6", None)]
#[case::no_digits("gpt", None)]
#[case::other_family("claude-3-5-sonnet", None)]
#[case::empty_suffix("openai/", None)]
fn gpt_version_reads_major_and_optional_minor(
    #[case] model: &str,
    #[case] expected: Option<(u64, u64)>,
) {
    assert_eq!(gpt_version(model), expected);
}

#[rstest]
#[case::below("gpt-5", false)]
#[case::older_family("gpt-4.5", false)]
#[case::minor_below("gpt-5.5", false)]
#[case::exactly_minimum("gpt-5.6", true)]
#[case::minor_above("gpt-5.7", true)]
#[case::two_digit_minor("gpt-5.10", true)]
#[case::next_major("gpt-6", true)]
#[case::provider_prefix("openai/gpt-5.6", true)]
#[case::upper_case("GPT-5.6", true)]
#[case::other_family("claude-3-5-sonnet", false)]
fn supports_openai_breakpoint_compares_versions(#[case] model: &str, #[case] expected: bool) {
    let facts = Facts::new(&[], None);
    assert_eq!(supports_openai_breakpoint(model, &facts), expected);
}

#[rstest]
#[case::full_name(&[("my-model", true)], "my-model", Some(true), true)]
#[case::suffix_after_slash(&[("my-model", true)], "openai/my-model", Some(true), true)]
#[case::full_name_before_suffix(&[("openai/my-model", false), ("my-model", true)], "openai/my-model", Some(false), false)]
#[case::full_name_true_wins(&[("openai/my-model", true), ("my-model", false)], "openai/my-model", Some(true), true)]
#[case::missing_entry(&[], "my-model", None, false)]
#[case::map_overrides_a_supported_version(&[("gpt-5.6", false)], "gpt-5.6", Some(false), false)]
#[case::map_enables_an_old_model(&[("gpt-4", true)], "gpt-4", Some(true), true)]
#[case::missing_entry_falls_back_to_the_version(&[], "gpt-5.6", None, true)]
#[case::missing_entry_and_old_version(&[], "gpt-4", None, false)]
fn model_map_breakpoint_flag_looks_up_the_full_name_first(
    #[case] flags: &[(&str, bool)],
    #[case] model: &str,
    #[case] expected_flag: Option<bool>,
    #[case] expected_support: bool,
) {
    let facts = Facts::new(flags, None);
    assert_eq!(model_map_breakpoint_flag(model, &facts), expected_flag);
    assert_eq!(supports_openai_breakpoint(model, &facts), expected_support);
}

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::parameter_is_openai(Some("https://api.openai.com/v1"), None, None, &[], true)]
#[case::exact_host(Some("https://api.openai.com"), None, None, &[], true)]
#[case::subdomain(Some("https://proxy.api.openai.com/v1"), None, None, &[], true)]
#[case::upper_case_host(Some("https://API.OpenAI.com/v1"), None, None, &[], true)]
#[case::port_is_ignored(Some("http://api.openai.com:8443/v1"), None, None, &[], true)]
#[case::lookalike_suffix(Some("https://notapi.openai.com/v1"), None, None, &[], false)]
#[case::openai_as_a_prefix(Some("https://api.openai.com.attacker.example"), None, None, &[], false)]
#[case::other_host(Some("https://gateway.example/v1"), None, None, &[], false)]
#[case::emptied_parameter_falls_to_base_url(Some(""), Some("https://api.openai.com/v1"), None, &[], true)]
#[case::base_url_precedes_the_global(Some(""), Some("https://gateway.example"), Some("https://api.openai.com"), &[], false)]
#[case::global_is_used(Some(""), None, Some("https://api.openai.com"), &[], true)]
#[case::environment_base_url(Some(""), None, None, &[("OPENAI_BASE_URL", "https://api.openai.com")], true)]
#[case::environment_api_base(Some(""), None, None, &[("OPENAI_API_BASE", "https://proxy.api.openai.com")], true)]
#[case::environment_base_url_precedes_api_base(Some(""), None, None, &[("OPENAI_BASE_URL", "https://api.openai.com"), ("OPENAI_API_BASE", "https://gateway.example")], true)]
#[case::environment_base_url_before_api_base(Some(""), None, None, &[("OPENAI_BASE_URL", "https://gateway.example"), ("OPENAI_API_BASE", "https://api.openai.com")], false)]
#[case::nothing_configured_targets_openai(Some(""), None, None, &[], true)]
fn targets_openai_api_follows_the_base_chain(
    #[case] api_base: Option<&str>,
    #[case] base_url: Option<&str>,
    #[case] global_api_base: Option<&str>,
    #[case] variables: &[(&str, &str)],
    #[case] expected: bool,
) {
    let environment = environment(variables);
    let mut inputs = dialect("gpt-5.6", &environment);
    inputs.api_base = api_base;
    inputs.base_url = base_url;
    inputs.global_api_base = global_api_base;
    assert_eq!(targets_openai_api(&inputs), expected);
}

#[rstest]
#[case::openai_provider(None, Some("openai"), true)]
#[case::declared_openai_provider(Some("openai"), None, true)]
#[case::declared_provider_wins(Some("anthropic"), Some("openai"), false)]
#[case::empty_declaration_falls_to_resolution(Some(""), Some("openai"), true)]
#[case::unresolvable_provider(None, None, false)]
#[case::other_provider(Some("azure"), None, false)]
fn targets_openai_breakpoint_needs_the_openai_provider(
    #[case] custom_llm_provider: Option<&str>,
    #[case] resolved_provider: Option<&str>,
    #[case] expected: bool,
) {
    let environment = environment(&[]);
    let mut inputs = dialect("gpt-5.6", &environment);
    inputs.custom_llm_provider = custom_llm_provider;
    inputs.api_base = Some("https://api.openai.com/v1");
    let facts = Facts::new(&[], resolved_provider);
    assert_eq!(targets_openai_breakpoint(&inputs, &facts), expected);
}

#[test]
fn targets_openai_breakpoint_accepts_a_non_openai_base_with_cache_options() {
    let environment = environment(&[]);
    let mut inputs = dialect("gpt-5.6", &environment);
    inputs.custom_llm_provider = Some("openai");
    inputs.api_base = Some("https://gateway.example/v1");
    let options = json("{}");
    inputs.prompt_cache_options = Some(&options);
    let facts = Facts::new(&[], None);
    assert!(targets_openai_breakpoint(&inputs, &facts));
}

#[test]
fn targets_openai_breakpoint_rejects_a_model_without_support() {
    let environment = environment(&[]);
    let mut inputs = dialect("gpt-5", &environment);
    inputs.custom_llm_provider = Some("openai");
    inputs.api_base = Some("https://api.openai.com/v1");
    let facts = Facts::new(&[], None);
    assert!(!targets_openai_breakpoint(&inputs, &facts));
}

#[test]
fn targets_openai_breakpoint_ignores_a_null_prompt_cache_options() {
    let environment = environment(&[]);
    let mut inputs = dialect("gpt-5.6", &environment);
    inputs.custom_llm_provider = Some("openai");
    inputs.api_base = Some("https://gateway.example/v1");
    let options = json("null");
    inputs.prompt_cache_options = Some(&options);
    let facts = Facts::new(&[], None);
    assert!(!targets_openai_breakpoint(&inputs, &facts));
}
