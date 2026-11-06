#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rstest::rstest;

use super::error::ValidationError;
use super::family::{conflicting_span_scope_error, cross_entry_family_error};
use super::options::{arize_sampling_rate_error, callback_config_error};
use super::parse::{parse_team_callback, validate_logging_metadata, validate_team_callback};
use super::{CallbackVars, FeatureFlags};

type Pairs = &'static [(&'static str, &'static str)];

enum Scenario {
    Config(Option<&'static str>, Pairs, bool),
    Arize(Option<&'static str>, Pairs),
    Family(Pairs, &'static [Pairs]),
    Scope(Pairs, &'static [Pairs]),
    Metadata(&'static str, bool),
    Team(&'static str, bool),
    TeamType(&'static str),
    TypedRate(&'static str),
    ManyScoped(usize),
}

fn vars(pairs: &[(&str, &str)]) -> CallbackVars {
    pairs
        .iter()
        .map(|(var, value)| ((*var).to_owned(), (*value).to_owned()))
        .collect()
}

fn outcome<E>(result: Result<(), E>) -> String
where
    E: ToString,
{
    result.map_or_else(|error| error.to_string(), |()| "ok".to_owned())
}

fn flags(v2: bool) -> FeatureFlags {
    FeatureFlags {
        otel_v2_enabled: v2,
    }
}

fn many_scoped_metadata_outcome(count: usize) -> String {
    let entry = r#"{"callback_vars": {"langfuse_span_scope": "full"}}"#;
    let json = format!(r#"{{"logging": [{}]}}"#, vec![entry; count].join(","));
    let started = Instant::now();
    let result = outcome(validate_logging_metadata(json.as_bytes(), flags(false)));
    // the quadratic scan takes minutes here, so a regression trips this bound
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "the span-scope check took {:?} for {count} entries",
        started.elapsed()
    );
    result
}

fn run(scenario: &Scenario) -> String {
    match scenario {
        Scenario::Config(name, pairs, v2) => {
            outcome(callback_config_error(*name, &vars(pairs), flags(*v2)))
        }
        Scenario::Arize(name, pairs) => outcome(arize_sampling_rate_error(*name, &vars(pairs))),
        Scenario::Family(incoming, entries) => {
            let held: Vec<CallbackVars> = entries.iter().map(|entry| vars(entry)).collect();
            outcome(cross_entry_family_error(&vars(incoming), &held))
        }
        Scenario::Scope(incoming, entries) => {
            let held: Vec<CallbackVars> = entries.iter().map(|entry| vars(entry)).collect();
            outcome(conflicting_span_scope_error(&vars(incoming), &held))
        }
        Scenario::Metadata(json, v2) => {
            outcome(validate_logging_metadata(json.as_bytes(), flags(*v2)))
        }
        Scenario::TeamType(json) => format!(
            "{:?}",
            parse_team_callback(json.as_bytes()).unwrap().callback_type
        ),
        Scenario::TypedRate(value) => {
            let pairs = vars(&[("arize_error_sampling_rate", value)]);
            match arize_sampling_rate_error(Some("arize"), &pairs) {
                Err(ValidationError::SamplingRateOutOfRange { rate, .. }) => rate.to_string(),
                other => format!("unexpected {other:?}"),
            }
        }
        Scenario::ManyScoped(count) => many_scoped_metadata_outcome(*count),
        Scenario::Team(json, v2) => outcome(validate_team_callback(json.as_bytes(), flags(*v2))),
    }
}

const RATE: &str = " must be a number between 0.0 and 1.0 (inclusive), got ";
const SCOPE_CONFLICT: &str = "langfuse_span_scope is already set to 'full' by another callback entry. Every entry shares one scope: remove that entry or send the same value.";
const NO_KEY: &str =
    "newrelic_region requires newrelic_api_key; the region rides the team's own key.";
const NEEDS_V2: &str =
    "Per-team New Relic routing requires the proxy to run with LITELLM_OTEL_V2=true.";

fn rate(var: &str, value: &str) -> String {
    format!("{var}{RATE}'{value}'")
}

fn held(family: &str, var: &str) -> String {
    format!(
        "{family} is already configured by another callback entry on this team. Remove that entry before setting {var} here."
    )
}

#[rstest]
#[case::empty_vars_skip_every_check(Scenario::Config(Some("arize"), &[], true), "ok")]
#[case::rate_zero_is_inclusive(Scenario::Config(Some("arize"), &[("arize_success_sampling_rate", "0")], false), "ok")]
#[case::rate_one_is_inclusive(Scenario::Config(Some("arize"), &[("arize_error_sampling_rate", "1.0")], false), "ok")]
#[case::rate_exponent_form(Scenario::Config(Some("arize"), &[("arize_error_sampling_rate", "5e-1")], false), "ok")]
#[case::rate_tiny_exponent_is_in_range(Scenario::Config(Some("arize"), &[("arize_error_sampling_rate", "1e-30")], false), "ok")]
#[case::rate_underflowing_exponent_is_in_range(Scenario::Config(Some("arize"), &[("arize_error_sampling_rate", "1e-400")], false), "ok")]
#[case::rate_tiny_negative_exponent_is_below_zero(Scenario::Arize(Some("arize"), &[("arize_error_sampling_rate", "-1e-30")]), &rate("arize_error_sampling_rate", "-1e-30"))]
#[case::rate_padded_with_spaces(Scenario::Config(Some("arize"), &[("arize_error_sampling_rate", " 0.25 ")], false), "ok")]
#[case::rate_empty_and_none_text_are_unset(Scenario::Config(Some("langfuse"), &[("arize_error_sampling_rate", ""), ("arize_success_sampling_rate", "None")], false), "ok")]
#[case::rate_above_one(Scenario::Arize(Some("arize"), &[("arize_success_sampling_rate", "1.0001")]), &rate("arize_success_sampling_rate", "1.0001"))]
#[case::rate_below_zero(Scenario::Arize(Some("arize"), &[("arize_error_sampling_rate", "-0.1")]), &rate("arize_error_sampling_rate", "-0.1"))]
#[case::rate_not_a_number(Scenario::Arize(Some("arize"), &[("arize_error_sampling_rate", "abc")]), &rate("arize_error_sampling_rate", "abc"))]
#[case::rate_nan(Scenario::Arize(Some("arize"), &[("arize_error_sampling_rate", "nan")]), &rate("arize_error_sampling_rate", "nan"))]
#[case::rate_empty_negative_exponent(Scenario::Arize(Some("arize"), &[("arize_error_sampling_rate", "1e-")]), &rate("arize_error_sampling_rate", "1e-"))]
#[case::rate_text_negative_exponent(Scenario::Arize(Some("arize"), &[("arize_error_sampling_rate", "1e-x")]), &rate("arize_error_sampling_rate", "1e-x"))]
#[case::rate_tiny_negative_plain_decimal_is_below_zero(Scenario::Arize(Some("arize"), &[("arize_error_sampling_rate", "-0.00000000000000000000000000001")]), &rate("arize_error_sampling_rate", "-0.00000000000000000000000000001"))]
#[case::rate_tiny_positive_plain_decimal_is_in_range(Scenario::Arize(Some("arize"), &[("arize_error_sampling_rate", "0.00000000000000000000000000001")]), "ok")]
#[case::rate_negative_zero_is_in_range(Scenario::Arize(Some("arize"), &[("arize_error_sampling_rate", "-0.000")]), "ok")]
#[case::rate_overflowing_exponent(Scenario::Arize(Some("arize"), &[("arize_error_sampling_rate", "1e400")]), &rate("arize_error_sampling_rate", "1e400"))]
#[case::rate_decimal_not_rounded_to_float(Scenario::Arize(Some("arize"), &[("arize_error_sampling_rate", "1.00000000000000001")]), &rate("arize_error_sampling_rate", "1.00000000000000001"))]
#[case::rate_error_var_is_checked_first(Scenario::Arize(Some("arize"), &[("arize_success_sampling_rate", "2"), ("arize_error_sampling_rate", "3")]), &rate("arize_error_sampling_rate", "3"))]
#[case::rate_on_other_callback(Scenario::Arize(Some("langfuse"), &[("arize_error_sampling_rate", "0.5")]), "arize_error_sampling_rate applies to the arize callback only, not 'langfuse'")]
#[case::rate_without_callback_name(Scenario::Arize(None, &[("arize_success_sampling_rate", "0.5")]), "arize_success_sampling_rate applies to the arize callback only, not None")]
#[case::rate_callback_checked_before_value(Scenario::Arize(Some("datadog"), &[("arize_error_sampling_rate", "oops")]), "arize_error_sampling_rate applies to the arize callback only, not 'datadog'")]
#[case::environment_valid(Scenario::Config(Some("langfuse"), &[("langfuse_environment", "prod-1_x")], false), "ok")]
#[case::environment_uppercase(Scenario::Config(Some("langfuse"), &[("langfuse_environment", "Prod")], false), "Invalid langfuse_environment 'Prod': must be lowercase alphanumerics/hyphens/underscores and must not start with 'langfuse'")]
#[case::environment_langfuse_prefix(Scenario::Config(Some("langfuse"), &[("langfuse_environment", "langfuse-dev")], false), "Invalid langfuse_environment 'langfuse-dev': must be lowercase alphanumerics/hyphens/underscores and must not start with 'langfuse'")]
#[case::environment_empty(Scenario::Config(None, &[("langfuse_environment", "")], false), "Invalid langfuse_environment '': must be lowercase alphanumerics/hyphens/underscores and must not start with 'langfuse'")]
#[case::environment_with_space(Scenario::Config(Some("langfuse"), &[("langfuse_environment", "a b")], false), "Invalid langfuse_environment 'a b': must be lowercase alphanumerics/hyphens/underscores and must not start with 'langfuse'")]
#[case::arize_error_wins_over_environment(Scenario::Config(Some("langfuse"), &[("langfuse_environment", "BAD"), ("arize_error_sampling_rate", "0.5")], false), "arize_error_sampling_rate applies to the arize callback only, not 'langfuse'")]
#[case::span_scope_full(Scenario::Config(Some("langfuse_otel"), &[("langfuse_span_scope", "full")], false), "ok")]
#[case::span_scope_llm_only(Scenario::Config(Some("langfuse_otel"), &[("langfuse_span_scope", "llm_only")], false), "ok")]
#[case::span_scope_other_callback(Scenario::Config(Some("langfuse"), &[("langfuse_span_scope", "full")], false), "langfuse_span_scope applies to the langfuse_otel callback only, not 'langfuse'")]
#[case::span_scope_no_callback(Scenario::Config(None, &[("langfuse_span_scope", "full")], false), "langfuse_span_scope applies to the langfuse_otel callback only, not None")]
#[case::span_scope_unknown_value(Scenario::Config(Some("langfuse_otel"), &[("langfuse_span_scope", "Full")], false), "Invalid langfuse_span_scope 'Full': must be one of ['full', 'llm_only']")]
#[case::environment_wins_over_span_scope(Scenario::Config(Some("langfuse"), &[("langfuse_span_scope", "full"), ("langfuse_environment", "X")], false), "Invalid langfuse_environment 'X': must be lowercase alphanumerics/hyphens/underscores and must not start with 'langfuse'")]
#[case::newrelic_other_prefix_is_ignored(Scenario::Config(Some("newrelic"), &[("dd_api_key", "k")], false), "ok")]
#[case::newrelic_needs_the_otel_v2_flag(Scenario::Config(Some("newrelic"), &[("newrelic_api_key", "k")], false), NEEDS_V2)]
#[case::newrelic_key_only(Scenario::Config(Some("newrelic"), &[("newrelic_api_key", "k")], true), "ok")]
#[case::newrelic_region_with_key(Scenario::Config(Some("newrelic"), &[("newrelic_region", "EU"), ("newrelic_api_key", "k")], true), "ok")]
#[case::newrelic_unknown_region(Scenario::Config(Some("newrelic"), &[("newrelic_region", "ap"), ("newrelic_api_key", "k")], true), "Unknown newrelic_region 'ap'. Supported regions: eu, us.")]
#[case::newrelic_region_without_key(Scenario::Config(Some("newrelic"), &[("newrelic_region", "us")], true), NO_KEY)]
#[case::newrelic_region_with_empty_key(Scenario::Config(Some("newrelic"), &[("newrelic_region", "us"), ("newrelic_api_key", "")], true), NO_KEY)]
#[case::newrelic_region_with_none_key(Scenario::Config(Some("newrelic"), &[("newrelic_region", "us"), ("newrelic_api_key", "None")], true), NO_KEY)]
#[case::newrelic_vars_on_other_callback(Scenario::Config(Some("langfuse"), &[("newrelic_region", "zz")], false), "ok")]
#[case::family_no_stored_entries(Scenario::Family(&[("langfuse_host", "https://a")], &[]), "ok")]
#[case::family_repeat_is_allowed(Scenario::Family(&[("langfuse_host", "https://a")], &[&[("langfuse_host", "https://a")]]), "ok")]
#[case::family_host_cannot_move(Scenario::Family(&[("langfuse_host", "https://evil")], &[&[("langfuse_host", "https://a")]]), &held("Langfuse", "langfuse_host"))]
#[case::family_alias_same_value(Scenario::Family(&[("langfuse_secret_key", "s")], &[&[("langfuse_secret", "s")]]), "ok")]
#[case::family_alias_other_value(Scenario::Family(&[("langfuse_secret_key", "t")], &[&[("langfuse_secret", "s")]]), &held("Langfuse", "langfuse_secret_key"))]
#[case::family_not_held_is_free(Scenario::Family(&[("dd_api_key", "k")], &[&[("langfuse_host", "h")]]), "ok")]
#[case::family_option_vars_have_no_family(Scenario::Family(&[("langfuse_span_scope", "llm_only"), ("arize_error_sampling_rate", "0.2")], &[&[("langfuse_host", "h"), ("arize_api_key", "k")]]), "ok")]
#[case::family_names_follow_the_prefix(Scenario::Family(&[("weave_api_key", "b")], &[&[("wandb_api_key", "a")]]), &held("Weights & Biases", "weave_api_key"))]
#[case::family_new_relic_label(Scenario::Family(&[("newrelic_region", "eu")], &[&[("newrelic_region", "us")]]), &held("New Relic", "newrelic_region"))]
#[case::family_gcs_label(Scenario::Family(&[("gcs_bucket_name", "b")], &[&[("gcs_bucket_name", "a")]]), &held("GCS", "gcs_bucket_name"))]
#[case::family_first_conflicting_var_wins(Scenario::Family(&[("dd_site", "x"), ("dd_api_key", "y")], &[&[("dd_api_key", "k"), ("dd_site", "s")]]), &held("Datadog", "dd_site"))]
#[case::family_unprefixed_var_is_free(Scenario::Family(&[("turn_off_message_logging", "true")], &[&[("langfuse_host", "h")]]), "ok")]
#[case::family_last_stored_value_wins(Scenario::Family(&[("posthog_host", "b")], &[&[("posthog_host", "a")], &[("posthog_host", "b")]]), "ok")]
#[case::scope_without_incoming(Scenario::Scope(&[("langfuse_host", "h")], &[&[("langfuse_span_scope", "full")]]), "ok")]
#[case::scope_same_value(Scenario::Scope(&[("langfuse_span_scope", "full")], &[&[("langfuse_span_scope", "full")]]), "ok")]
#[case::scope_other_value(Scenario::Scope(&[("langfuse_span_scope", "llm_only")], &[&[("langfuse_host", "h")], &[("langfuse_span_scope", "full")]]), SCOPE_CONFLICT)]
#[case::metadata_without_logging(Scenario::Metadata(r#"{"other": 1}"#, false), "ok")]
#[case::metadata_null_or_empty(Scenario::Metadata("null", false), "ok")]
#[case::metadata_logging_not_a_list(Scenario::Metadata(r#"{"logging": "arize"}"#, false), "ok")]
#[case::metadata_skips_odd_entries(Scenario::Metadata(r#"{"logging": [1, {"callback_name": 3, "callback_vars": {"arize_error_sampling_rate": "9"}}, {"callback_name": "arize", "callback_vars": "x"}]}"#, false), "ok")]
#[case::metadata_reports_entry_error(Scenario::Metadata(r#"{"logging": [{"callback_name": "arize", "callback_vars": {"arize_error_sampling_rate": "9"}}]}"#, false), &rate("arize_error_sampling_rate", "9"))]
#[case::metadata_numbers_become_text(Scenario::Metadata(r#"{"logging": [{"callback_name": "arize", "callback_vars": {"arize_error_sampling_rate": 0.5}}]}"#, false), "ok")]
#[case::metadata_out_of_range_number(Scenario::Metadata(r#"{"logging": [{"callback_name": "arize", "callback_vars": {"arize_error_sampling_rate": 7}}]}"#, false), &rate("arize_error_sampling_rate", "7"))]
#[case::metadata_null_key_counts_as_absent(Scenario::Metadata(r#"{"logging": [{"callback_name": "newrelic", "callback_vars": {"newrelic_region": "us", "newrelic_api_key": null}}]}"#, true), NO_KEY)]
#[case::metadata_scope_conflict_across_entries(Scenario::Metadata(r#"{"logging": [{"callback_name": "langfuse_otel", "callback_vars": {"langfuse_span_scope": "full"}}, {"callback_name": "langfuse_otel", "callback_vars": {"langfuse_span_scope": "llm_only"}}]}"#, false), SCOPE_CONFLICT)]
#[case::metadata_entry_errors_come_before_conflicts(Scenario::Metadata(r#"{"logging": [{"callback_name": "langfuse_otel", "callback_vars": {"langfuse_span_scope": "full"}}, {"callback_name": "langfuse_otel", "callback_vars": {"langfuse_span_scope": "bad"}}]}"#, false), "Invalid langfuse_span_scope 'bad': must be one of ['full', 'llm_only']")]
#[case::metadata_conflict_needs_no_callback_name(Scenario::Metadata(r#"{"logging": [{"callback_vars": {"langfuse_span_scope": "full"}}, {"callback_vars": {"langfuse_span_scope": "x"}}]}"#, false), SCOPE_CONFLICT)]
#[case::metadata_first_scoped_entry_is_the_reference(Scenario::Metadata(r#"{"logging": [{"callback_name": "langfuse_otel", "callback_vars": {"langfuse_host": "h"}}, {"callback_vars": {"langfuse_span_scope": "full"}}, {"callback_vars": {"langfuse_span_scope": "llm_only"}}]}"#, false), SCOPE_CONFLICT)]
#[case::metadata_boolean_variable_becomes_text(Scenario::Metadata(r#"{"logging": [{"callback_name": "langfuse", "callback_vars": {"turn_off_message_logging": true}}]}"#, false), "ok")]
#[case::metadata_boolean_rate_is_text(Scenario::Metadata(r#"{"logging": [{"callback_name": "arize", "callback_vars": {"arize_error_sampling_rate": false}}]}"#, false), &rate("arize_error_sampling_rate", "False"))]
#[case::metadata_many_entries_share_one_scope(Scenario::ManyScoped(100_000), "ok")]
#[case::metadata_list_variable_is_rejected(
    Scenario::Metadata(
        r#"{"logging": [{"callback_name": "arize", "callback_vars": {"arize_api_key": ["k"]}}]}"#,
        false
    ),
    "callback variable arize_api_key must be a string, a number or null"
)]
#[case::metadata_not_json(
    Scenario::Metadata("{", false),
    "the callback config is not valid JSON of the expected shape"
)]
#[case::metadata_not_an_object(
    Scenario::Metadata("[]", false),
    "the callback config is not valid JSON of the expected shape"
)]
#[case::team_valid(
    Scenario::Team(
        r#"{"callback_name": "arize", "callback_vars": {"arize_error_sampling_rate": "0.5"}}"#,
        false
    ),
    "ok"
)]
#[case::team_validates_vars(Scenario::Team(r#"{"callback_name": "langfuse", "callback_type": "failure", "callback_vars": {"langfuse_environment": "Bad"}}"#, false), "Invalid langfuse_environment 'Bad': must be lowercase alphanumerics/hyphens/underscores and must not start with 'langfuse'")]
#[case::team_unknown_type(
    Scenario::Team(
        r#"{"callback_name": "arize", "callback_type": "both", "callback_vars": {}}"#,
        false
    ),
    "the callback config is not valid JSON of the expected shape"
)]
#[case::team_missing_vars(
    Scenario::Team(r#"{"callback_name": "arize"}"#, false),
    "the callback config is not valid JSON of the expected shape"
)]
#[case::team_type_defaults_when_absent(
    Scenario::TeamType(r#"{"callback_name": "a", "callback_vars": {}}"#),
    "SuccessAndFailure"
)]
#[case::team_type_defaults_when_null(
    Scenario::TeamType(r#"{"callback_name": "a", "callback_type": null, "callback_vars": {}}"#),
    "SuccessAndFailure"
)]
#[case::team_type_failure(
    Scenario::TeamType(
        r#"{"callback_name": "a", "callback_type": "failure", "callback_vars": {}}"#
    ),
    "Failure"
)]
#[case::rate_error_keeps_the_decimal(Scenario::TypedRate("1.50"), "1.50")]
fn callback_validation_behavior_tests_behavior(#[case] scenario: Scenario, #[case] expected: &str) {
    assert_eq!(run(&scenario), expected);
}

#[rstest]
fn replays_the_callback_config_corpus(#[files("fuzz/corpus/callback_config/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let flags = FeatureFlags {
        otel_v2_enabled: true,
    };
    let _metadata = validate_logging_metadata(&bytes, flags);
    let _team = validate_team_callback(&bytes, flags);
}
