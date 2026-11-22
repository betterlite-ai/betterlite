use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::MAX_JSON_BYTES;
use rstest::rstest;

use crate::shadow_eval_validation::error::ShadowEvalRequestError;
use crate::shadow_eval_validation::request::parse_shadow_eval_request;

use super::support::{parse, validation_field};

fn refused(body: &str) -> bool {
    parse_shadow_eval_request(body.as_bytes()).is_err()
}

#[rstest]
#[case(0_usize, true)]
#[case(1_usize, false)]
#[case(100_usize, false)]
#[case(101_usize, true)]
fn the_target_count_is_bounded(#[case] count: usize, #[case] rejected: bool) {
    let ids: Vec<String> = (0..count).map(|index| format!("team-{index}")).collect();
    let body = format!(
        r#"{{"team_ids":{},"router_names":["r"],"shadow_percentage":50}}"#,
        serde_json::to_string(&ids).unwrap()
    );
    assert_eq!(refused(&body), rejected);
    if count == 0 || count == 101 {
        let error = parse_shadow_eval_request(body.as_bytes()).unwrap_err();
        assert_eq!(validation_field(error), "api_key_ids");
    }
}

#[test]
fn the_target_ceiling_counts_every_kind_together() {
    let teams = vec!["t"; 100];
    let body = format!(
        r#"{{"api_key_ids":["k"],"team_ids":{},"router_names":["r"],"shadow_percentage":50}}"#,
        serde_json::to_string(&teams).unwrap()
    );
    let error = parse_shadow_eval_request(body.as_bytes()).unwrap_err();
    assert_eq!(validation_field(error), "api_key_ids");
}

#[rstest]
#[case("0.09", true)]
#[case("0.1", false)]
#[case("100", false)]
#[case("100.1", true)]
fn the_percentage_is_bounded(#[case] value: &str, #[case] rejected: bool) {
    let body = format!(r#"{{"team_ids":["t"],"router_names":["r"],"shadow_percentage":{value}}}"#);
    assert_eq!(refused(&body), rejected);
    if rejected {
        let error = parse_shadow_eval_request(body.as_bytes()).unwrap_err();
        assert_eq!(validation_field(error), "shadow_percentage");
    }
}

#[rstest]
#[case(0_u32, true)]
#[case(1_u32, false)]
#[case(30_u32, false)]
#[case(31_u32, true)]
fn the_duration_is_bounded(#[case] days: u32, #[case] rejected: bool) {
    let body = format!(
        r#"{{"team_ids":["t"],"router_names":["r"],"shadow_percentage":50,"duration_days":{days}}}"#
    );
    assert_eq!(refused(&body), rejected);
    if rejected {
        let error = parse_shadow_eval_request(body.as_bytes()).unwrap_err();
        assert_eq!(validation_field(error), "duration_days");
    }
}

#[test]
fn a_fractional_duration_is_rejected() {
    let error = parse_shadow_eval_request(
        br#"{"team_ids":["t"],"router_names":["r"],"shadow_percentage":50,"duration_days":7.5}"#,
    )
    .unwrap_err();
    assert_eq!(validation_field(error), "duration_days");
}

#[rstest]
#[case("0.009", true)]
#[case("0.01", false)]
#[case("1e-2", false)]
#[case("10000", false)]
#[case("10000.01", true)]
fn the_budget_is_bounded(#[case] value: &str, #[case] rejected: bool) {
    let body = format!(
        r#"{{"team_ids":["t"],"router_names":["r"],"shadow_percentage":50,"max_budget":{value}}}"#
    );
    assert_eq!(refused(&body), rejected);
    if rejected {
        let error = parse_shadow_eval_request(body.as_bytes()).unwrap_err();
        assert_eq!(validation_field(error), "max_budget");
    }
}

#[test]
fn keeps_the_budget_decimal_as_written() {
    let parsed = parse(
        r#"{"team_ids":["t"],"router_names":["r"],"shadow_percentage":50,"max_budget":1e-2}"#,
    );
    assert_eq!(parsed.max_budget, rust_decimal::Decimal::new(1, 2));
}

#[rstest]
#[case(4_usize, false)]
#[case(5_usize, true)]
fn router_names_are_bounded_before_dedup(#[case] count: usize, #[case] rejected: bool) {
    let names = vec!["r"; count];
    let body = format!(
        r#"{{"team_ids":["t"],"router_names":{},"shadow_percentage":50}}"#,
        serde_json::to_string(&names).unwrap()
    );
    assert_eq!(refused(&body), rejected);
    if rejected {
        let error = parse_shadow_eval_request(body.as_bytes()).unwrap_err();
        assert_eq!(validation_field(error), "router_names");
    }
}

#[test]
fn four_duplicate_router_names_dedupe_to_one() {
    let parsed =
        parse(r#"{"team_ids":["t"],"router_names":["r","r","r","r"],"shadow_percentage":50}"#);
    assert_eq!(parsed.router_names, ["r"]);
}

#[rstest]
#[case(100_usize, false)]
#[case(101_usize, true)]
fn models_are_bounded_before_dedup(#[case] count: usize, #[case] rejected: bool) {
    let models = vec!["m"; count];
    let body = format!(
        r#"{{"team_ids":["t"],"router_names":["r"],"models":{},"shadow_percentage":50}}"#,
        serde_json::to_string(&models).unwrap()
    );
    assert_eq!(refused(&body), rejected);
    if rejected {
        let error = parse_shadow_eval_request(body.as_bytes()).unwrap_err();
        assert_eq!(validation_field(error), "models");
    }
}

#[rstest]
#[case(r#"["m"]"#, false)]
#[case(r#"[""]"#, true)]
#[case(r#"["  "]"#, true)]
fn model_names_are_never_blank(#[case] models: &str, #[case] rejected: bool) {
    let body = format!(
        r#"{{"team_ids":["t"],"router_names":["r"],"models":{models},"shadow_percentage":50}}"#
    );
    assert_eq!(refused(&body), rejected);
}

#[rstest]
#[case(None)]
#[case(Some("null"))]
fn the_percentage_is_required(#[case] percentage: Option<&str>) {
    let field = match percentage {
        Some(value) => format!(r#","shadow_percentage":{value}"#),
        None => String::new(),
    };
    let body = format!(r#"{{"team_ids":["t"],"router_names":["r"]{field}}}"#);
    let error = parse_shadow_eval_request(body.as_bytes()).unwrap_err();
    assert_eq!(validation_field(error), "shadow_percentage");
}

#[test]
fn accepts_a_body_of_exactly_the_byte_limit() {
    let mut bytes =
        br#"{"api_key_ids":["k"],"router_names":["r"],"shadow_percentage":50}"#.to_vec();
    bytes.resize(MAX_JSON_BYTES, b' ');
    parse_shadow_eval_request(&bytes).unwrap();
}

#[test]
fn refuses_a_body_one_byte_past_the_limit_with_its_size() {
    let mut bytes =
        br#"{"api_key_ids":["k"],"router_names":["r"],"shadow_percentage":50}"#.to_vec();
    bytes.resize(MAX_JSON_BYTES.saturating_add(1), b' ');
    let error = parse_shadow_eval_request(&bytes).unwrap_err();
    match error {
        ShadowEvalRequestError::Bounded(PrivateJsonError::TooLarge(size, limit)) => {
            assert_eq!(size, MAX_JSON_BYTES.saturating_add(1));
            assert_eq!(limit, MAX_JSON_BYTES);
        }
        other => assert_eq!(other.to_string(), "expected the byte bound"),
    }
}

#[test]
fn a_wrongly_typed_field_names_its_path() {
    let error = parse_shadow_eval_request(
        br#"{"api_key_ids":"key-1","router_names":["r"],"shadow_percentage":50}"#,
    )
    .unwrap_err();
    match error {
        ShadowEvalRequestError::Field { path, .. } => assert_eq!(path, "api_key_ids"),
        other => assert_eq!(other.to_string(), "expected a field error"),
    }
}

#[test]
fn a_deeply_nested_body_is_refused_before_decoding() {
    let depth = 70_usize;
    let body = format!(
        r#"{{"api_key_ids":["k"],"router_names":["r"],"shadow_percentage":50,"models":{}{}}}"#,
        "[".repeat(depth),
        "]".repeat(depth)
    );
    let error = parse_shadow_eval_request(body.as_bytes()).unwrap_err();
    match error {
        ShadowEvalRequestError::Bounded(PrivateJsonError::TooDeep(limit)) => {
            assert_eq!(limit, 64);
        }
        other => assert_eq!(other.to_string(), "expected the depth bound"),
    }
}
