use rstest::rstest;

use super::support::{guardrail, settings};
use crate::panw_prisma_airs::audit::Direction;
use crate::panw_prisma_airs::error_detail::build_error_detail;
use crate::panw_prisma_airs::verdict::ScanVerdict;

const BLOCK_BODY: &[u8] = br#"{"action":"block","category":"malicious","scan_id":"s1","prompt_masked_data":{"data":"x"},"response_masked_data":{"data":"y"},"_internal":1,"message":"clash","report_id":"r1"}"#;

fn detail_json(template: Option<&str>, body: &[u8], direction: Direction) -> String {
    let mut config = settings("http://unused.test");
    config.violation_message_template = template.map(str::to_owned);
    let verdict = ScanVerdict::from_body(body).expect("verdict parses");
    let detail = build_error_detail(&guardrail(config), &verdict, direction);
    serde_json::to_string(&detail).expect("detail serializes")
}

#[test]
fn a_blocked_request_names_the_prompt_and_the_request_code() {
    assert_eq!(
        detail_json(None, BLOCK_BODY, Direction::Request),
        r#"{"error":{"action":"block","scan_id":"s1","prompt_masked_data":{"data":"x"},"report_id":"r1","message":"Prompt blocked by PANW Prisma AI Security policy (Category: malicious)","type":"guardrail_violation","code":"panw_prisma_airs_blocked","guardrail":"panw","category":"malicious"}}"#
    );
}

#[test]
fn a_blocked_response_names_the_response_and_the_response_code() {
    let json = detail_json(None, BLOCK_BODY, Direction::Response);
    assert!(json.contains(
        r#""message":"Response blocked by PANW Prisma AI Security policy (Category: malicious)""#
    ));
    assert!(json.contains(r#""code":"panw_prisma_airs_response_blocked""#));
}

#[rstest]
#[case::request(Direction::Request)]
#[case::response(Direction::Response)]
fn the_model_generation_never_reaches_the_client(#[case] direction: Direction) {
    let json = detail_json(None, BLOCK_BODY, direction);
    assert!(!json.contains("response_masked_data"), "{json}");
    assert!(
        json.contains(r#""prompt_masked_data":{"data":"x"}"#),
        "{json}"
    );
}

#[test]
fn private_scan_fields_and_body_keys_are_not_echoed() {
    let json = detail_json(None, BLOCK_BODY, Direction::Request);
    assert!(!json.contains("_internal"));
    assert!(!json.contains("clash"));
}

#[test]
fn a_verdict_without_a_category_is_reported_as_unknown() {
    let json = detail_json(None, br#"{"action":"block"}"#, Direction::Request);
    assert!(json.contains(r#""category":"unknown""#));
    assert!(json.contains("(Category: unknown)"));
}

#[rstest]
#[case::every_field(
    "{guardrail_name}/{category}/{action_type}/{default_message}",
    "panw/malicious/Prompt/Prompt blocked by PANW Prisma AI Security policy (Category: malicious)"
)]
#[case::literal_braces("{{category}} is {category}", "{category} is malicious")]
#[case::plain_text("Blocked by policy.", "Blocked by policy.")]
fn the_violation_template_is_rendered_with_the_scan_context(
    #[case] template: &str,
    #[case] expected: &str,
) {
    let json = detail_json(Some(template), BLOCK_BODY, Direction::Request);
    assert!(
        json.contains(&format!(r#""message":"{expected}""#)),
        "{json}"
    );
}

#[rstest]
#[case::unknown_key("{nope}")]
#[case::unclosed_brace("{category")]
#[case::stray_closing_brace("category}")]
fn a_template_that_does_not_format_falls_back_to_the_default_message(#[case] template: &str) {
    let json = detail_json(Some(template), BLOCK_BODY, Direction::Request);
    assert!(
        json.contains(
            r#""message":"Prompt blocked by PANW Prisma AI Security policy (Category: malicious)""#
        ),
        "{json}"
    );
}
