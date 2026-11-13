#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rstest::rstest;

use super::config::{Calibration, Config, ResponseFormat};
use super::error::ForecastError;
use super::forecast::route;
use super::prompt::{SYSTEM_PROMPT, response_format, schema, system_prompt};
use super::reply::{MAX_REPLY_BYTES, SCAN_BUDGET, first_object, parse_cap_reply};
use super::verdict::{Boundary, Rule};

const JSON_SCHEMA_FORMAT: &str = r#"{"type":"json_schema","json_schema":{"name":"CapabilityClassifierDecision","strict":true,"schema":{"type":"object","additionalProperties":false,"required":["crux","primary_rule","capability_boundary","p_solve"],"properties":{"crux":{"type":"string","minLength":1},"primary_rule":{"type":"string","enum":["SUP-1","SUP-2","SUP-3","SUP-4","SUP-5","UNC-1","UNC-2","LIM-1","LIM-2","none"]},"capability_boundary":{"type":"string","enum":["supported","uncertain","unsupported","unmatched"]},"p_solve":{"type":"number","minimum":0.0,"maximum":1.0}}}}}"#;

fn number(text: &str) -> f64 {
    text.parse().unwrap()
}

fn drafting(base_threshold: f64, threshold_step: f64) -> Config {
    Config {
        efficient_tier: "SIMPLE".to_owned(),
        capable_tier: "REASONING".to_owned(),
        base_threshold,
        threshold_step,
        max_output_tokens: 4096,
        calibration: None,
        response_format: ResponseFormat::JsonSchema,
    }
}

fn draft_calibrated(calibration: Calibration) -> Config {
    Config {
        calibration: Some(calibration),
        ..drafting(0.5_f64, 0.0_f64)
    }
}

fn configured(base_threshold: f64, threshold_step: f64) -> Config {
    drafting(base_threshold, threshold_step).validate().unwrap()
}

fn calibrated(calibration: Calibration) -> Config {
    draft_calibrated(calibration).validate().unwrap()
}

fn calibration(version: &str, slope: f64, intercept: f64) -> Calibration {
    Calibration {
        version: version.to_owned(),
        slope,
        intercept,
    }
}

fn verdict_json(rule: &str, boundary: &str, p_solve: &str) -> String {
    format!(
        r#"{{"crux":"the hard part","primary_rule":"{rule}","capability_boundary":"{boundary}","p_solve":{p_solve}}}"#
    )
}

fn routed(base_threshold: f64, threshold_step: f64, content: &str) -> String {
    route(
        &configured(base_threshold, threshold_step),
        content.as_bytes(),
    )
    .unwrap()
    .tier
}

#[derive(Debug, Clone, Copy)]
enum Behavior {
    ResponseFormatSchema,
    ResponseFormatObject,
    SystemPromptSchema,
    SystemPromptObject,
    PlainReply,
    FencedReply,
    FencedLanguage,
    ProseBefore,
    ProseAfter,
    FencedThenProse,
    FirstCompleteObjectWins,
    NonFiniteObjectWins,
    EveryRuleBoundaryPair,
    CalibrationApplied,
    CalibrationEndpoints,
}

#[rstest]
#[case::response_format_schema(Behavior::ResponseFormatSchema)]
#[case::response_format_object(Behavior::ResponseFormatObject)]
#[case::system_prompt_schema(Behavior::SystemPromptSchema)]
#[case::system_prompt_object(Behavior::SystemPromptObject)]
#[case::plain_reply(Behavior::PlainReply)]
#[case::fenced_reply(Behavior::FencedReply)]
#[case::fenced_language(Behavior::FencedLanguage)]
#[case::prose_before(Behavior::ProseBefore)]
#[case::prose_after(Behavior::ProseAfter)]
#[case::fenced_then_prose(Behavior::FencedThenProse)]
#[case::first_complete_object_wins(Behavior::FirstCompleteObjectWins)]
#[case::non_finite_object_wins(Behavior::NonFiniteObjectWins)]
#[case::every_rule_boundary_pair(Behavior::EveryRuleBoundaryPair)]
#[case::calibration_applied(Behavior::CalibrationApplied)]
#[case::calibration_endpoints(Behavior::CalibrationEndpoints)]
fn capability_forecast_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::ResponseFormatSchema => assert_response_format_schema(),
        Behavior::ResponseFormatObject => assert_response_format_object(),
        Behavior::SystemPromptSchema => assert_system_prompt_schema(),
        Behavior::SystemPromptObject => assert_system_prompt_object(),
        Behavior::PlainReply => assert_accepted_verdict(&verdict_json("SUP-1", "supported", "0.8")),
        Behavior::FencedReply => assert_accepted_verdict(&format!(
            "  ```\n{}\n```  ",
            verdict_json("SUP-1", "supported", "0.8")
        )),
        Behavior::FencedLanguage => assert_accepted_verdict(&format!(
            "```json\n{}\n```",
            verdict_json("SUP-1", "supported", "0.8")
        )),
        Behavior::ProseBefore => assert_accepted_verdict(&format!(
            "Sure {{here}} is the verdict you asked for:\n\n{}",
            verdict_json("SUP-1", "supported", "0.8")
        )),
        Behavior::ProseAfter => assert_accepted_verdict(&format!(
            "{}\n\nThe efficient solver should handle this {{well}}.",
            verdict_json("SUP-1", "supported", "0.8")
        )),
        Behavior::FencedThenProse => assert_accepted_verdict(&format!(
            "```json\n{}\n```\n\n## Reasoning\n\nThe task is coupled, so the forecasts differ.",
            verdict_json("SUP-1", "supported", "0.8")
        )),
        Behavior::FirstCompleteObjectWins => assert_first_object_wins(),
        Behavior::NonFiniteObjectWins => assert_non_finite_object_wins(),
        Behavior::EveryRuleBoundaryPair => assert_every_rule_boundary_pair(),
        Behavior::CalibrationApplied => assert_calibration_applied(),
        Behavior::CalibrationEndpoints => assert_calibration_endpoints(),
    }
}

fn assert_response_format_schema() {
    let rendered = serde_json::to_string(&response_format(ResponseFormat::JsonSchema)).unwrap();
    assert_eq!(rendered, JSON_SCHEMA_FORMAT);
}

fn assert_response_format_object() {
    let rendered = serde_json::to_string(&response_format(ResponseFormat::JsonObject)).unwrap();
    assert_eq!(rendered, r#"{"type":"json_object"}"#);
}

fn assert_system_prompt_schema() {
    let prompt = system_prompt(ResponseFormat::JsonSchema).unwrap();
    assert_eq!(prompt, SYSTEM_PROMPT);
    assert_eq!(prompt.len(), 4556);
}

fn assert_system_prompt_object() {
    let prompt = system_prompt(ResponseFormat::JsonObject).unwrap();
    let sorted = serde_json::to_value(schema()).unwrap();
    let schema = serde_json::to_string_pretty(&sorted).unwrap();
    assert_eq!(prompt.len(), 5358);
    assert!(prompt.starts_with(SYSTEM_PROMPT));
    // sort_keys=True puts additionalProperties before properties, unlike the packaged literal
    assert!(prompt.contains("\"additionalProperties\": false,\n  \"properties\": {"));
    assert!(prompt.ends_with(&schema));
}

fn assert_accepted_verdict(content: &str) {
    let parsed = parse_cap_reply(content.as_bytes()).unwrap();
    assert_eq!(parsed.crux, "the hard part");
    assert_eq!(parsed.primary_rule, Rule::Sup1);
    assert_eq!(parsed.capability_boundary, Boundary::Supported);
    assert_eq!(parsed.p_solve.to_bits(), number("0.8").to_bits());
}

fn assert_first_object_wins() {
    let reply = format!(
        "{{\"note\": 1}}\n{}",
        verdict_json("SUP-1", "supported", "0.8")
    );
    let error = parse_cap_reply(reply.as_bytes()).unwrap_err();
    assert!(matches!(error, ForecastError::VerdictSchema { .. }));
}

fn assert_non_finite_object_wins() {
    // Python's raw_decode accepts these literals, so the first object is the candidate and verdict validation refuses it
    let nan = format!(
        "{{\"crux\":\"nan\",\"primary_rule\":\"SUP-1\",\"capability_boundary\":\"supported\",\"p_solve\":NaN}}\n{}",
        verdict_json("SUP-1", "supported", "0.8")
    );
    let nan_error = parse_cap_reply(nan.as_bytes()).unwrap_err();
    assert!(matches!(nan_error, ForecastError::VerdictSchema { .. }));
    let infinity = format!(
        "{{\"crux\":\"infinite\",\"primary_rule\":\"SUP-1\",\"capability_boundary\":\"supported\",\"p_solve\":Infinity}}\n{}",
        verdict_json("SUP-1", "supported", "0.8")
    );
    let infinity_error = parse_cap_reply(infinity.as_bytes()).unwrap_err();
    assert!(matches!(
        infinity_error,
        ForecastError::VerdictSchema { .. }
    ));
    let negative = format!(
        "{{\"crux\":\"negative\",\"primary_rule\":\"SUP-1\",\"capability_boundary\":\"supported\",\"p_solve\":-Infinity}}\n{}",
        verdict_json("SUP-1", "supported", "0.8")
    );
    let negative_error = parse_cap_reply(negative.as_bytes()).unwrap_err();
    assert!(matches!(
        negative_error,
        ForecastError::VerdictSchema { .. }
    ));
    let quoted =
        verdict_json("SUP-1", "supported", "0.5").replace("the hard part", "NaN and Infinity");
    assert_eq!(
        parse_cap_reply(quoted.as_bytes()).unwrap().crux,
        "NaN and Infinity"
    );
    let nested = r#"{"crux":"x","primary_rule":"SUP-1","capability_boundary":"supported","p_solve":0.5,"extra":{"values":[NaN,Infinity,-Infinity]}}"#;
    let error = parse_cap_reply(nested.as_bytes()).unwrap_err();
    assert!(matches!(error, ForecastError::VerdictSchema { .. }));
}

fn assert_pair(rule: Rule, boundary: Boundary) {
    let rule_name = serde_json::to_string(&rule).unwrap();
    let boundary_name = serde_json::to_string(&boundary).unwrap();
    let reply = format!(
        r#"{{"crux":"x","primary_rule":{rule_name},"capability_boundary":{boundary_name},"p_solve":0.5}}"#
    );
    let parsed = parse_cap_reply(reply.as_bytes()).unwrap();
    assert_eq!(parsed.primary_rule, rule);
    assert_eq!(parsed.capability_boundary, boundary);
}

fn assert_every_rule_boundary_pair() {
    assert_pair(Rule::Sup1, Boundary::Supported);
    assert_pair(Rule::Sup2, Boundary::Supported);
    assert_pair(Rule::Sup3, Boundary::Supported);
    assert_pair(Rule::Sup4, Boundary::Supported);
    assert_pair(Rule::Sup5, Boundary::Supported);
    assert_pair(Rule::Unc1, Boundary::Uncertain);
    assert_pair(Rule::Unc2, Boundary::Uncertain);
    assert_pair(Rule::Lim1, Boundary::Unsupported);
    assert_pair(Rule::Lim2, Boundary::Unsupported);
    assert_pair(Rule::None, Boundary::Unmatched);
}

fn assert_calibration_applied() {
    let config = calibrated(calibration("held-out-v1", 2.0_f64, 1.0_f64));
    let decided = route(
        &config,
        verdict_json("SUP-1", "supported", "0.5").as_bytes(),
    )
    .unwrap();
    assert_eq!(
        decided.forecast.p_solve.to_bits(),
        number("0.7310585786300049").to_bits()
    );
    assert_eq!(
        decided.forecast.calibration_version.as_deref(),
        Some("held-out-v1")
    );
    assert_eq!(decided.tier, "SIMPLE");
}

fn assert_calibration_endpoints() {
    let identity = calibrated(calibration("v1", 1.0_f64, 0.0_f64));
    let low = route(
        &identity,
        verdict_json("SUP-1", "supported", "0").as_bytes(),
    )
    .unwrap();
    assert_eq!(
        low.forecast.p_solve.to_bits(),
        number("1.0000000000000008e-06").to_bits()
    );
    let high = route(
        &identity,
        verdict_json("SUP-1", "supported", "1").as_bytes(),
    )
    .unwrap();
    assert_eq!(
        high.forecast.p_solve.to_bits(),
        number("0.999999").to_bits()
    );
    let zero = calibrated(calibration("v1", 0.0_f64, 0.0_f64));
    let flat = route(&zero, verdict_json("SUP-1", "supported", "0.25").as_bytes()).unwrap();
    assert_eq!(flat.forecast.p_solve.to_bits(), number("0.5").to_bits());
    let steep = calibrated(calibration("v1", 20.0_f64, -20.0_f64));
    let floor = route(&steep, verdict_json("SUP-1", "supported", "0").as_bytes()).unwrap();
    let middle = route(&steep, verdict_json("SUP-1", "supported", "0.5").as_bytes()).unwrap();
    let ceiling = route(&steep, verdict_json("SUP-1", "supported", "1").as_bytes()).unwrap();
    assert!(floor.forecast.p_solve.is_finite());
    assert!(middle.forecast.p_solve.is_finite());
    assert!(ceiling.forecast.p_solve.is_finite());
    assert!((0.0_f64..=1.0_f64).contains(&floor.forecast.p_solve));
    assert!((0.0_f64..=1.0_f64).contains(&middle.forecast.p_solve));
    assert!((0.0_f64..=1.0_f64).contains(&ceiling.forecast.p_solve));
    assert!(floor.forecast.p_solve < middle.forecast.p_solve);
    assert!(middle.forecast.p_solve < ceiling.forecast.p_solve);
}

#[rstest]
#[case::epsilon_inclusive(assert_epsilon_inclusive)]
#[case::decimal_rounding(assert_decimal_rounding)]
#[case::solve_range(assert_solve_range)]
#[case::rule_boundary_pair(assert_rule_boundary_pair)]
#[case::schema_shape(assert_schema_shape)]
#[case::base_threshold(assert_base_threshold)]
#[case::threshold_step(assert_threshold_step)]
#[case::threshold_sum(assert_threshold_sum)]
#[case::token_count(assert_token_count)]
#[case::tier_names(assert_tier_names)]
#[case::calibration_version(assert_calibration_version)]
#[case::calibration_coefficients(assert_calibration_coefficients)]
#[case::reply_byte_bound(assert_reply_byte_bound)]
#[case::scan_budget(assert_scan_budget)]
#[case::reply_encoding(assert_reply_encoding)]
#[case::selection(assert_selection)]
fn capability_forecast_boundaries(#[case] check: fn()) {
    check();
}

fn assert_epsilon_inclusive() {
    let at_epsilon = routed(
        number("0.5000000000000002"),
        0.0_f64,
        &verdict_json("SUP-1", "supported", "0.5"),
    );
    assert_eq!(at_epsilon, "SIMPLE");
    let past_epsilon = routed(
        number("0.5000000000000004"),
        0.0_f64,
        &verdict_json("SUP-1", "supported", "0.5"),
    );
    assert_eq!(past_epsilon, "REASONING");
    let above = routed(0.5_f64, 0.0_f64, &verdict_json("SUP-1", "supported", "0.6"));
    assert_eq!(above, "SIMPLE");
}

fn assert_decimal_rounding() {
    let config = configured(0.1_f64, 0.1_f64);
    let reply = verdict_json("LIM-1", "unsupported", "0.3");
    let decided = route(&config, reply.as_bytes()).unwrap();
    assert_eq!(
        decided.forecast.threshold.to_bits(),
        number("0.30000000000000004").to_bits()
    );
    assert_eq!(decided.tier, "SIMPLE");
    let below = routed(
        0.1_f64,
        0.1_f64,
        &verdict_json("LIM-1", "unsupported", "0.2999"),
    );
    assert_eq!(below, "REASONING");
}

fn assert_solve_range() {
    let zero = routed(0.0_f64, 0.0_f64, &verdict_json("SUP-1", "supported", "0"));
    assert_eq!(zero, "SIMPLE");
    let one = routed(1.0_f64, 0.0_f64, &verdict_json("SUP-1", "supported", "1"));
    assert_eq!(one, "SIMPLE");
    let negative = parse_cap_reply(verdict_json("SUP-1", "supported", "-0.0001").as_bytes());
    assert!(matches!(
        negative.unwrap_err(),
        ForecastError::SolveProbability { .. }
    ));
    let above = parse_cap_reply(verdict_json("SUP-1", "supported", "1.0001").as_bytes());
    assert!(matches!(
        above.unwrap_err(),
        ForecastError::SolveProbability { .. }
    ));
    // the object is located first, then the typed decode refuses the non-finite or out-of-range value
    let nan = parse_cap_reply(verdict_json("SUP-1", "supported", "NaN").as_bytes()).unwrap_err();
    assert!(matches!(nan, ForecastError::VerdictSchema { .. }));
    let infinite =
        parse_cap_reply(verdict_json("SUP-1", "supported", "Infinity").as_bytes()).unwrap_err();
    assert!(matches!(infinite, ForecastError::VerdictSchema { .. }));
    let overflow =
        parse_cap_reply(verdict_json("SUP-1", "supported", "1e999").as_bytes()).unwrap_err();
    assert!(matches!(overflow, ForecastError::VerdictSchema { .. }));
}

fn assert_rule_boundary_pair() {
    let mismatch = parse_cap_reply(verdict_json("SUP-1", "unsupported", "0.5").as_bytes());
    assert!(matches!(
        mismatch.unwrap_err(),
        ForecastError::RuleBoundary {
            rule: Rule::Sup1,
            expected: Boundary::Supported,
            found: Boundary::Unsupported,
        }
    ));
    let empty = verdict_json("none", "unmatched", "0.5").replace("the hard part", "");
    assert!(matches!(
        parse_cap_reply(empty.as_bytes()).unwrap_err(),
        ForecastError::BlankCrux
    ));
    let spaces = verdict_json("none", "unmatched", "0.5").replace("the hard part", "   ");
    assert!(matches!(
        parse_cap_reply(spaces.as_bytes()).unwrap_err(),
        ForecastError::BlankCrux
    ));
}

fn assert_schema_shape() {
    let extra = r#"{"crux":"x","primary_rule":"none","capability_boundary":"unmatched","p_solve":0.5,"route":"efficient"}"#;
    assert!(matches!(
        parse_cap_reply(extra.as_bytes()).unwrap_err(),
        ForecastError::VerdictSchema { .. }
    ));
    let missing = r#"{"crux":"x","primary_rule":"none","capability_boundary":"unmatched"}"#;
    assert!(matches!(
        parse_cap_reply(missing.as_bytes()).unwrap_err(),
        ForecastError::VerdictSchema { .. }
    ));
    let unknown = verdict_json("SUP-9", "supported", "0.5");
    assert!(matches!(
        parse_cap_reply(unknown.as_bytes()).unwrap_err(),
        ForecastError::VerdictSchema { .. }
    ));
    // Python keeps the last duplicate key; the derived typed decoder refuses the reply and fails closed
    let duplicate = r#"{"crux":"x","crux":"y","primary_rule":"none","capability_boundary":"unmatched","p_solve":0.5}"#;
    assert!(matches!(
        parse_cap_reply(duplicate.as_bytes()).unwrap_err(),
        ForecastError::VerdictSchema { .. }
    ));
}

fn assert_base_threshold() {
    let negative = drafting(-0.1_f64, 0.0_f64).validate();
    assert!(matches!(
        negative.unwrap_err(),
        ForecastError::BaseThreshold { .. }
    ));
    let above = drafting(1.1_f64, 0.0_f64).validate();
    assert!(matches!(
        above.unwrap_err(),
        ForecastError::BaseThreshold { .. }
    ));
    let not_a_number = drafting(f64::NAN, 0.0_f64).validate();
    assert!(matches!(
        not_a_number.unwrap_err(),
        ForecastError::BaseThreshold { .. }
    ));
    let infinite = drafting(f64::INFINITY, 0.0_f64).validate();
    assert!(matches!(
        infinite.unwrap_err(),
        ForecastError::BaseThreshold { .. }
    ));
    assert_eq!(
        configured(0.0_f64, 0.0_f64).base_threshold.to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(
        configured(1.0_f64, 0.0_f64).base_threshold.to_bits(),
        1.0_f64.to_bits()
    );
}

fn assert_threshold_step() {
    let negative = drafting(0.5_f64, -0.1_f64).validate();
    assert!(matches!(
        negative.unwrap_err(),
        ForecastError::ThresholdStep { .. }
    ));
    let not_a_number = drafting(0.5_f64, f64::NAN).validate();
    assert!(matches!(
        not_a_number.unwrap_err(),
        ForecastError::ThresholdStep { .. }
    ));
    let infinite = drafting(0.5_f64, f64::INFINITY).validate();
    assert!(matches!(
        infinite.unwrap_err(),
        ForecastError::ThresholdStep { .. }
    ));
    assert_eq!(
        configured(0.5_f64, 0.25_f64).threshold_step.to_bits(),
        0.25_f64.to_bits()
    );
}

fn assert_threshold_sum() {
    assert_eq!(
        configured(0.5_f64, 0.25_f64).threshold_step.to_bits(),
        0.25_f64.to_bits()
    );
    let above = drafting(0.5_f64, number("0.2500000000000001")).validate();
    assert!(matches!(above.unwrap_err(), ForecastError::ThresholdRange));
}

fn assert_token_count() {
    let mut none_allowed = configured(0.5_f64, 0.0_f64);
    none_allowed.max_output_tokens = 0;
    assert!(matches!(
        none_allowed.validate().unwrap_err(),
        ForecastError::MaxOutputTokens
    ));
    let mut one_token = configured(0.5_f64, 0.0_f64);
    one_token.max_output_tokens = 1;
    assert_eq!(one_token.validate().unwrap().max_output_tokens, 1);
}

fn assert_tier_names() {
    let trimmed = Config {
        efficient_tier: "  SIMPLE  ".to_owned(),
        capable_tier: "  REASONING  ".to_owned(),
        ..drafting(0.5_f64, 0.0_f64)
    }
    .validate()
    .unwrap();
    assert_eq!(trimmed.efficient_tier, "SIMPLE");
    assert_eq!(trimmed.capable_tier, "REASONING");
    let empty_efficient = Config {
        efficient_tier: "   ".to_owned(),
        ..drafting(0.5_f64, 0.0_f64)
    }
    .validate();
    assert!(matches!(
        empty_efficient.unwrap_err(),
        ForecastError::EmptyEfficientTier
    ));
    let empty_capable = Config {
        capable_tier: String::new(),
        ..drafting(0.5_f64, 0.0_f64)
    }
    .validate();
    assert!(matches!(
        empty_capable.unwrap_err(),
        ForecastError::EmptyCapableTier
    ));
}

fn rejects_version(version: &str) -> bool {
    matches!(
        draft_calibrated(calibration(version, 1.0_f64, 0.0_f64)).validate(),
        Err(ForecastError::CalibrationVersion)
    )
}

fn assert_calibration_version() {
    assert!(rejects_version(""));
    assert!(rejects_version(" "));
    assert!(rejects_version("v1 "));
    assert!(rejects_version(" v1"));
    assert!(rejects_version("a\nb"));
    let longest = "v".repeat(128);
    assert!(
        draft_calibrated(calibration(&longest, 1.0_f64, 0.0_f64))
            .validate()
            .unwrap()
            .calibration
            .is_some()
    );
    let too_long = "v".repeat(129);
    assert!(rejects_version(&too_long));
}

fn calibration_error(slope: f64, intercept: f64) -> ForecastError {
    draft_calibrated(calibration("v1", slope, intercept))
        .validate()
        .unwrap_err()
}

fn rejects_slope(slope: f64) -> bool {
    matches!(
        calibration_error(slope, 0.0_f64),
        ForecastError::CalibrationSlope { .. }
    )
}

fn rejects_intercept(intercept: f64) -> bool {
    matches!(
        calibration_error(1.0_f64, intercept),
        ForecastError::CalibrationIntercept { .. }
    )
}

fn assert_calibration_coefficients() {
    assert!(rejects_slope(-1.0_f64));
    assert!(rejects_slope(20.5_f64));
    assert!(rejects_slope(f64::NAN));
    assert!(rejects_slope(f64::INFINITY));
    assert!(rejects_intercept(-20.5_f64));
    assert!(rejects_intercept(20.5_f64));
    assert!(rejects_intercept(f64::NAN));
    assert!(rejects_intercept(f64::NEG_INFINITY));
    assert!(
        draft_calibrated(calibration("v1", 0.0_f64, -20.0_f64))
            .validate()
            .unwrap()
            .calibration
            .is_some()
    );
    assert!(
        draft_calibrated(calibration("v1", 20.0_f64, 20.0_f64))
            .validate()
            .unwrap()
            .calibration
            .is_some()
    );
}

fn assert_reply_byte_bound() {
    let verdict = verdict_json("SUP-1", "supported", "0.5");
    let padding = " ".repeat(MAX_REPLY_BYTES.saturating_sub(verdict.len()));
    let mut at_limit = verdict;
    at_limit.push_str(&padding);
    assert_eq!(at_limit.len(), MAX_REPLY_BYTES);
    let parsed = parse_cap_reply(at_limit.as_bytes()).unwrap();
    assert_eq!(parsed.crux, "the hard part");
    let past_limit = format!("{at_limit} ");
    let error = parse_cap_reply(past_limit.as_bytes()).unwrap_err();
    assert!(matches!(
        error,
        ForecastError::ReplyTooLarge { bytes, limit }
            if bytes == MAX_REPLY_BYTES.saturating_add(1) && limit == MAX_REPLY_BYTES
    ));
}

fn assert_scan_budget() {
    assert_eq!(SCAN_BUDGET, 0x0100_0000);
    let started = Instant::now();
    let flood = "{\"a\":".repeat(209_715);
    let error = parse_cap_reply(flood.as_bytes()).unwrap_err();
    assert!(matches!(error, ForecastError::NoVerdictObject));
    assert!(started.elapsed() < Duration::from_secs(10));
    // fast failures are charged what they read, so their remaining bytes do not exhaust an explicit budget
    let fast = format!("{}{}", "{x ".repeat(300), "{}");
    assert_eq!(first_object(&fast, 600), Some("{}"));
    // one fast failure past the budget stops the walk before the object
    let over = format!("{}{}", "{x ".repeat(301), "{}");
    assert_eq!(first_object(&over, 600), None);
    // a failure that reads to the end of the reply is still charged the whole remainder, not a couple of bytes
    let unterminated = format!("{}{}", "{\"a\": {}", " ".repeat(90));
    assert_eq!(first_object(&unterminated, 97), None);
    assert_eq!(first_object(&unterminated, 98), Some("{}"));
    // the same holds when the failed start crosses newlines, so the line offset belongs in the charge
    let multiline = format!("{}{}", "{\"a\":\n\n {}", " ".repeat(10));
    assert_eq!(
        first_object(&multiline, multiline.len().saturating_sub(1)),
        None
    );
    assert_eq!(first_object(&multiline, multiline.len()), Some("{}"));
    // the worst-case charge would refuse this reply at the byte limit, while its failed starts read 2 bytes each
    let verdict = verdict_json("SUP-1", "supported", "0.8");
    let padding = " ".repeat(
        MAX_REPLY_BYTES
            .saturating_sub(60)
            .saturating_sub(verdict.len()),
    );
    let large = format!("{}{padding}{verdict}", "{x ".repeat(20));
    assert_eq!(large.len(), MAX_REPLY_BYTES);
    assert_eq!(
        parse_cap_reply(large.as_bytes()).unwrap().primary_rule,
        Rule::Sup1
    );
}

fn assert_reply_encoding() {
    assert!(matches!(
        parse_cap_reply(&[0xff, 0xfe]).unwrap_err(),
        ForecastError::ReplyNotText { .. }
    ));
    assert!(matches!(
        parse_cap_reply(b"").unwrap_err(),
        ForecastError::NoVerdictObject
    ));
    assert!(matches!(
        parse_cap_reply(b"no object here").unwrap_err(),
        ForecastError::NoVerdictObject
    ));
}

fn assert_selection() {
    let config = configured(0.5_f64, 0.1_f64);
    let exact = route(
        &config,
        verdict_json("UNC-1", "uncertain", "0.6").as_bytes(),
    )
    .unwrap();
    assert_eq!(exact.forecast.threshold.to_bits(), number("0.6").to_bits());
    assert_eq!(exact.tier, "SIMPLE");
    assert!(exact.meets_threshold);
    let below = route(
        &config,
        verdict_json("UNC-1", "uncertain", "0.599999").as_bytes(),
    )
    .unwrap();
    assert_eq!(below.tier, "REASONING");
    assert!(!below.meets_threshold);
    let unsupported = route(
        &config,
        verdict_json("LIM-1", "unsupported", "0.7").as_bytes(),
    )
    .unwrap();
    assert_eq!(
        unsupported.forecast.threshold.to_bits(),
        number("0.7").to_bits()
    );
    assert_eq!(unsupported.tier, "SIMPLE");
    assert!(unsupported.meets_threshold);
    assert_eq!(unsupported.forecast.calibration_version, None);
}

#[rstest]
fn replays_the_cap_reply_corpus(#[files("fuzz/corpus/cap_reply/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let outcome = parse_cap_reply(&bytes);
    if name.contains("invalid") {
        assert!(outcome.is_err(), "{name} must not decode");
    } else {
        assert!(outcome.is_ok(), "{name} must decode");
    }
}
