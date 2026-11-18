use rstest::rstest;

use crate::complexity_scoring::tier::{ComplexityTier, Tier};

use super::super::error::ClassifierVerdictError;
use super::super::normalize::{OperatorSection, normalize_section};
use super::super::reply::{MAX_REPLY_BYTES, SCAN_BUDGET, first_object, parse_class_reply};
use super::super::rubric::{ClassificationRubric, calibration_examples_section};
use super::blank_tiers;

#[test]
fn the_reply_and_scan_limits_are_the_declared_numbers() {
    assert_eq!(MAX_REPLY_BYTES, 1_048_576);
    assert_eq!(SCAN_BUDGET, 16_777_216);
}

#[test]
fn the_reply_byte_limit_is_inclusive_and_counts_bytes() {
    let body = r#"{"tier":"SIMPLE"}"#;
    let mut padded = String::from(body);
    padded.push_str(&" ".repeat(MAX_REPLY_BYTES - body.len()));
    assert_eq!(padded.len(), MAX_REPLY_BYTES);
    assert_eq!(
        parse_class_reply(padded.as_bytes(), &blank_tiers())
            .unwrap()
            .tier,
        Tier::BuiltIn(ComplexityTier::Simple)
    );
    padded.push(' ');
    match parse_class_reply(padded.as_bytes(), &blank_tiers()) {
        Err(ClassifierVerdictError::ReplyTooLarge { bytes, limit }) => {
            assert_eq!(bytes, MAX_REPLY_BYTES + 1);
            assert_eq!(limit, MAX_REPLY_BYTES);
        }
        other => panic!("over-limit reply decoded: {other:?}"),
    }
    let multibyte = "é".repeat(MAX_REPLY_BYTES / 2);
    assert_eq!(multibyte.len(), MAX_REPLY_BYTES);
    assert!(matches!(
        parse_class_reply(multibyte.as_bytes(), &blank_tiers()),
        Err(ClassifierVerdictError::NoVerdictObject)
    ));
    let mut one_byte_over = multibyte.into_bytes();
    one_byte_over.push(b'a');
    assert!(matches!(
        parse_class_reply(&one_byte_over, &blank_tiers()),
        Err(ClassifierVerdictError::ReplyTooLarge { .. })
    ));
    assert!(matches!(
        parse_class_reply(&vec![0xff; MAX_REPLY_BYTES + 1], &blank_tiers()),
        Err(ClassifierVerdictError::ReplyTooLarge { .. })
    ));
}

#[rstest]
#[case("")]
#[case("   ")]
#[case("[]")]
#[case("\"SIMPLE\"")]
#[case(r#"{"tier": "SIMPLE""#)]
#[case("no json at all")]
fn a_reply_without_a_complete_object_is_rejected(#[case] reply: &str) {
    assert!(matches!(
        parse_class_reply(reply.as_bytes(), &blank_tiers()),
        Err(ClassifierVerdictError::NoVerdictObject)
    ));
}

#[test]
fn a_reply_that_is_not_text_is_rejected_before_scanning() {
    assert!(matches!(
        parse_class_reply(&[0xff, b'{'], &blank_tiers()),
        Err(ClassifierVerdictError::ReplyNotText { .. })
    ));
}

#[rstest]
#[case(r#"{"confidence": 0.9}"#)]
#[case(r#"{"tier": 7}"#)]
#[case(r#"{"tier": null}"#)]
#[case(r#"{"tier": ["SIMPLE"]}"#)]
#[case(r#"{"meta": {"tier": "COMPLEX"}}"#)]
fn a_schema_mismatch_is_rejected(#[case] reply: &str) {
    assert!(matches!(
        parse_class_reply(reply.as_bytes(), &blank_tiers()),
        Err(ClassifierVerdictError::VerdictSchema { .. })
    ));
}

#[test]
fn an_unrecognized_tier_keeps_the_raw_label() {
    match parse_class_reply(br#"{"tier": "  nope  "}"#, &blank_tiers()) {
        Err(ClassifierVerdictError::UnknownTier { label }) => assert_eq!(label, "  nope  "),
        other => panic!("unrecognized tier accepted: {other:?}"),
    }
}

#[rstest]
#[case(r#"{"tier": "SIMPLE", "p": NaN}"#)]
#[case(r#"{"tier": "SIMPLE", "p": Infinity}"#)]
#[case(r#"{"tier": "SIMPLE", "p": -Infinity}"#)]
#[case(r#"{"tier":"SIMPLE","p":NaN}"#)]
#[case(r#"{"tier":"SIMPLE","p":NaN,"q":1}"#)]
#[case(r#"{"tier":"SIMPLE","p":[NaN]}"#)]
#[case(r#"{"tier":"SIMPLE","p":NaN }"#)]
#[case(r#"NaN {"tier":"SIMPLE"}"#)]
fn constants_are_masked_only_where_a_value_could_start(#[case] reply: &str) {
    assert_eq!(
        parse_class_reply(reply.as_bytes(), &blank_tiers())
            .unwrap()
            .tier,
        Tier::BuiltIn(ComplexityTier::Simple),
        "{reply}"
    );
}

#[test]
fn constants_inside_a_string_or_a_value_keep_the_reply_invalid() {
    assert!(matches!(
        parse_class_reply(br#"{"tier": "NaN"}"#, &blank_tiers()),
        Err(ClassifierVerdictError::UnknownTier { label }) if label == "NaN"
    ));
    assert!(matches!(
        parse_class_reply(br#"{"tier": Infinity}"#, &blank_tiers()),
        Err(ClassifierVerdictError::VerdictSchema { .. })
    ));
    assert!(matches!(
        parse_class_reply(br#"{"tier":"SIMPLE","p":NaNx}"#, &blank_tiers()),
        Err(ClassifierVerdictError::VerdictSchema { .. })
    ));
    assert!(matches!(
        parse_class_reply(br#"{"tier":"SIMPLE","p":xInfinity}"#, &blank_tiers()),
        Err(ClassifierVerdictError::VerdictSchema { .. })
    ));
}

#[test]
fn the_scan_budget_cancels_a_pathological_reply() {
    // a fast failure is charged only the bytes it read, so 300 two-byte failures fit a 600-byte budget
    let fast = format!("{}{}", "{x ".repeat(300), "{}");
    assert_eq!(first_object(&fast, 600), Some("{}"));
    let over = format!("{}{}", "{x ".repeat(301), "{}");
    assert_eq!(first_object(&over, 600), None);
    let control = format!("{}{}", "{\"a\":{".repeat(1_000), r#"1{"tier":"SIMPLE"}"#);
    assert_eq!(
        parse_class_reply(control.as_bytes(), &blank_tiers())
            .unwrap()
            .tier,
        Tier::BuiltIn(ComplexityTier::Simple)
    );
    let cancelled = format!("{}{}", "{\"a\":{".repeat(30_000), r#"1{"tier":"SIMPLE"}"#);
    assert!(cancelled.len() <= MAX_REPLY_BYTES);
    assert!(matches!(
        parse_class_reply(cancelled.as_bytes(), &blank_tiers()),
        Err(ClassifierVerdictError::NoVerdictObject)
    ));
}

#[rstest]
#[case(OperatorSection::Prompt, 2000, "classification_prompt")]
#[case(OperatorSection::Examples, 4000, "classification_examples")]
fn operator_sections_are_stripped_and_capped_in_chars(
    #[case] section: OperatorSection,
    #[case] cap: usize,
    #[case] field: &str,
) {
    assert_eq!(section.name_and_cap(), (field, cap));
    assert_eq!(normalize_section(section, None).unwrap(), None);
    let at_cap = "é".repeat(cap);
    assert_eq!(
        normalize_section(section, Some(&format!("  {at_cap}  "))).unwrap(),
        Some(at_cap.clone())
    );
    let over = "é".repeat(cap + 1);
    match normalize_section(section, Some(&over)) {
        Err(ClassifierVerdictError::OperatorSectionTooLong {
            field: name,
            cap: reported,
        }) => {
            assert_eq!(name, field);
            assert_eq!(reported, cap);
        }
        other => panic!("over-cap section accepted: {other:?}"),
    }
    match normalize_section(section, Some(" \t\n ")) {
        Err(ClassifierVerdictError::OperatorSectionBlank { field: name }) => {
            assert_eq!(name, field)
        }
        other => panic!("blank section accepted: {other:?}"),
    }
}

#[test]
fn calibration_examples_require_every_included_tier() {
    assert!(
        calibration_examples_section(ClassificationRubric::Chat, &super::canonical())
            .unwrap()
            .contains("-> REASONING")
    );
    let partial = [
        (ComplexityTier::Simple, "SIMPLE"),
        (ComplexityTier::Medium, "MEDIUM"),
        (ComplexityTier::Complex, "COMPLEX"),
    ];
    assert!(matches!(
        calibration_examples_section(ClassificationRubric::Chat, &partial),
        Err(ClassifierVerdictError::MissingLabel { tier: "REASONING" })
    ));
}
