#![cfg(test)]

use base64::Engine as _;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use chrono::NaiveDateTime;
use rstest::rstest;

use crate::database_tokens::expiry::{
    ExpiryError, MAX_CLAIMS_DEPTH, MAX_TOKEN_BYTES, TokenAuthKind, decode_database_token_expiry,
    parse_token_expiration,
};
use crate::database_tokens::tests::support::capture;

fn naive(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").expect("the fixture timestamp parses")
}

fn rds_token(date: &str, expires: &str) -> String {
    format!(
        "postgresql://user:password@db.example:5432/litellm?X-Amz-Date={date}&X-Amz-Expires={expires}&X-Amz-Signature=abc"
    )
}

fn unpadded_claims_token(claims: &str) -> String {
    format!("header.{}.signature", URL_SAFE_NO_PAD.encode(claims))
}

fn padded_claims_token(claims: &str) -> String {
    format!("header.{}.signature", URL_SAFE.encode(claims))
}

fn assert_lenient_date(date: &str, expected: &str) {
    let token = rds_token(date, "+60");
    assert_eq!(
        parse_token_expiration(TokenAuthKind::Rds, &token).unwrap(),
        Some(naive(expected)),
        "{date}"
    );
}

fn assert_padding_matches(claims: &str) {
    let unpadded = unpadded_claims_token(claims);
    let padded = padded_claims_token(claims);
    let expected = parse_token_expiration(TokenAuthKind::Azure, &padded).unwrap();
    assert!(expected.is_some(), "{claims}");
    assert_eq!(
        parse_token_expiration(TokenAuthKind::Azure, &unpadded).unwrap(),
        expected,
        "{claims}"
    );
}

fn assert_claim_rejected(claims: &str) {
    let token = unpadded_claims_token(claims);
    let (result, logs) = capture(|| parse_token_expiration(TokenAuthKind::Azure, &token));
    assert_eq!(result.unwrap(), None, "{claims}");
    assert!(!logs.is_empty(), "{claims}");
}

fn assert_unreadable_debug(token: &str) {
    let (result, logs) = capture(|| parse_token_expiration(TokenAuthKind::Azure, token));
    assert_eq!(result.unwrap(), None, "{token}");
    assert!(
        logs.iter()
            .any(|log| log.contains("Failed to parse Azure Entra token expiration:")),
        "{logs:?}"
    );
}

#[test]
fn rds_expiry_first_values() {
    let duplicated = "postgresql://u:p@h:1/d?X-Amz-Date=&X-Amz-Date=20240101T000000Z&X-Amz-Expires=60&X-Amz-Expires=900";
    assert_eq!(
        parse_token_expiration(TokenAuthKind::Rds, duplicated).unwrap(),
        Some(naive("2024-01-01T00:01:00"))
    );
    // strptime reads a fixed-width year and 1-2 digit fields, and its literals are case-insensitive
    assert_lenient_date("2024101T000000Z", "2024-10-01T00:01:00");
    assert_lenient_date("20240101t000000z", "2024-01-01T00:01:00");
    assert_lenient_date("20240101T00000Z", "2024-01-01T00:01:00");
}

#[rstest]
#[case::no_question_mark("postgresql://user:token@db.example:5432/litellm")]
#[case::missing_expires("postgresql://u:p@h/d?X-Amz-Date=20240101T000000Z")]
#[case::missing_date("postgresql://u:p@h/d?X-Amz-Expires=60")]
#[case::blank_expires("postgresql://u:p@h/d?X-Amz-Date=20240101T000000Z&X-Amz-Expires=")]
fn rds_missing_fields_silent(#[case] token: &str) {
    let (result, logs) = capture(|| parse_token_expiration(TokenAuthKind::Rds, token));
    assert_eq!(result.unwrap(), None);
    assert!(logs.is_empty(), "{logs:?}");
}

#[rstest]
#[case::bad_month("20241301T000000Z", "60")]
#[case::bad_second("20240101T000060Z", "60")]
#[case::bad_integer("20240101T000000Z", "soon")]
#[case::huge_expiry("20240101T000000Z", "999999999999999999999999")]
#[case::past_year_9999("99991231T235959Z", "2")]
fn rds_invalid_fields_debug(#[case] date: &str, #[case] expires: &str) {
    let token = rds_token(date, expires);
    let (result, logs) = capture(|| parse_token_expiration(TokenAuthKind::Rds, &token));
    assert_eq!(result.unwrap(), None);
    assert!(
        logs.iter()
            .any(|log| log.contains("Failed to parse RDS IAM token expiration:")),
        "{logs:?}"
    );
}

#[test]
fn rds_negative_expiry() {
    let token = rds_token("20240101T000000Z", "-60");
    assert_eq!(
        parse_token_expiration(TokenAuthKind::Rds, &token).unwrap(),
        Some(naive("2023-12-31T23:59:00"))
    );
}

#[test]
fn encoded_rds_password_not_parsed() {
    // minting percent-encodes the whole token, so the password handed back still has no literal '?'
    let token = "postgresql://user:tok%3FX-Amz-Date%3D20240101T000000Z%26X-Amz-Expires%3D900@db.example:5432/db";
    let (result, logs) = capture(|| parse_token_expiration(TokenAuthKind::Rds, token));
    assert_eq!(result.unwrap(), None);
    assert!(logs.is_empty(), "{logs:?}");
}

#[rstest]
#[case::no_segments("")]
#[case::one_segment("abc")]
#[case::two_segments("a.b")]
#[case::four_segments("a.b.c.d")]
fn entra_segment_count_silent(#[case] token: &str) {
    let (result, logs) = capture(|| parse_token_expiration(TokenAuthKind::Azure, token));
    assert_eq!(result.unwrap(), None);
    assert!(logs.is_empty(), "{logs:?}");
}

#[test]
fn entra_base64_padding() {
    assert_padding_matches("{\"exp\":1}");
    assert_padding_matches("{\"exp\":99}");
    assert_padding_matches("{\"exp\":999}");
    assert_eq!(
        parse_token_expiration(
            TokenAuthKind::Azure,
            &unpadded_claims_token("{\"exp\":1700000000}")
        )
        .unwrap(),
        Some(naive("2023-11-14T22:13:20"))
    );
}

#[test]
fn entra_exp_required_and_extra_claims() {
    let extra =
        unpadded_claims_token("{\"exp\":1700000000,\"sub\":\"x\",\"scope\":[\"a\",{\"b\":1}]}");
    assert_eq!(
        parse_token_expiration(TokenAuthKind::Azure, &extra).unwrap(),
        Some(naive("2023-11-14T22:13:20"))
    );
    // pydantic keeps the last duplicate, so an unreadable earlier exp is overwritten and a later one is not
    let duplicates = unpadded_claims_token("{\"exp\":\"x\",\"exp\" : 1700000000 }");
    assert_eq!(
        parse_token_expiration(TokenAuthKind::Azure, &duplicates).unwrap(),
        Some(naive("2023-11-14T22:13:20"))
    );
    assert_claim_rejected("{\"exp\":1700000000,\"exp\":\"x\"}");
    assert_claim_rejected("[1]");
    assert_claim_rejected("{}");
    assert_claim_rejected("{\"exp\":null}");
    assert_claim_rejected("{\"exp\":true}");
    assert_claim_rejected("{\"exp\":1.5}");
}

#[test]
fn entra_invalid_claim_debug() {
    assert_unreadable_debug(&unpadded_claims_token("{not json}"));
    assert_unreadable_debug(&unpadded_claims_token("{\"exp\":}"));
    assert_unreadable_debug("header.####.signature");
    assert_unreadable_debug(&unpadded_claims_token("{\"pad\":1}"));
}

#[rstest]
#[case::last_valid_second("253402300799", Some("9999-12-31T23:59:59"))]
#[case::first_invalid_second("253402300800", None)]
#[case::i64_max("9223372036854775807", None)]
#[case::i64_min("-9223372036854775808", None)]
#[case::two_to_63("9223372036854775808", None)]
#[case::twenty_digits("99999999999999999999", None)]
#[case::large_positive("999999999999999999", None)]
#[case::before_epoch("-1", Some("1969-12-31T23:59:59"))]
fn entra_timestamp_overflow_propagates(#[case] exp: &str, #[case] expected: Option<&str>) {
    let token = unpadded_claims_token(&format!("{{\"exp\":{exp}}}"));
    let result = parse_token_expiration(TokenAuthKind::Azure, &token);
    match expected {
        Some(text) => assert_eq!(result.unwrap(), Some(naive(text))),
        None => assert!(matches!(result, Err(ExpiryError::TimestampOutOfRange))),
    }
}

#[test]
fn entra_expired_unsigned_token_is_hint() {
    // no signature is verified and no liveness is required: only the claim feeds the refresh scheduler
    let unsigned = format!(
        "unsigned-header.{}.unsigned-signature",
        URL_SAFE_NO_PAD.encode("{\"exp\":1}")
    );
    assert_eq!(
        parse_token_expiration(TokenAuthKind::Azure, &unsigned).unwrap(),
        Some(naive("1970-01-01T00:00:01"))
    );
}

#[test]
fn naive_utc_expiry() {
    let rds =
        parse_token_expiration(TokenAuthKind::Rds, &rds_token("20240101T000000Z", "900")).unwrap();
    let azure = parse_token_expiration(
        TokenAuthKind::Azure,
        &unpadded_claims_token("{\"exp\":1700000000}"),
    )
    .unwrap();
    assert_eq!(rds, Some(naive("2024-01-01T00:15:00")));
    assert_eq!(azure, Some(naive("2023-11-14T22:13:20")));
    assert_eq!(
        rds.map(|value| value.and_utc().to_string()),
        Some("2024-01-01 00:15:00 UTC".to_owned())
    );
}

#[test]
fn native_expiry_size_depth_limits() {
    let prefix = "postgresql://user:password@db.example:5432/litellm?X-Amz-Date=20240101T000000Z&X-Amz-Expires=60&pad=";
    let filler = "a".repeat(MAX_TOKEN_BYTES - prefix.len());
    let boundary = format!("{prefix}{filler}");
    assert_eq!(boundary.len(), MAX_TOKEN_BYTES);
    let (boundary_result, boundary_logs) =
        capture(|| decode_database_token_expiry(boundary.as_bytes(), TokenAuthKind::Rds));
    assert!(boundary_result.unwrap().is_some());
    assert!(boundary_logs.is_empty(), "{boundary_logs:?}");
    let longer = format!("{boundary}a");
    let (longer_result, longer_logs) =
        capture(|| decode_database_token_expiry(longer.as_bytes(), TokenAuthKind::Rds));
    assert_eq!(longer_result.unwrap(), None);
    assert!(
        longer_logs
            .iter()
            .any(|log| log.contains("Failed to parse RDS IAM token expiration:")),
        "{longer_logs:?}"
    );

    let mut accepted_nesting = "[".repeat(MAX_CLAIMS_DEPTH - 1);
    accepted_nesting.push_str(&"]".repeat(MAX_CLAIMS_DEPTH - 1));
    let accepted = unpadded_claims_token(&format!("{{\"exp\":1,\"pad\":{accepted_nesting}}}"));
    assert!(
        parse_token_expiration(TokenAuthKind::Azure, &accepted)
            .unwrap()
            .is_some()
    );
    let mut rejected_nesting = "[".repeat(MAX_CLAIMS_DEPTH);
    rejected_nesting.push_str(&"]".repeat(MAX_CLAIMS_DEPTH));
    let rejected = unpadded_claims_token(&format!("{{\"exp\":1,\"pad\":{rejected_nesting}}}"));
    let (rejected_result, rejected_logs) =
        capture(|| decode_database_token_expiry(rejected.as_bytes(), TokenAuthKind::Azure));
    assert_eq!(rejected_result.unwrap(), None);
    assert!(
        rejected_logs
            .iter()
            .any(|log| log.contains("Failed to parse Azure Entra token expiration:")),
        "{rejected_logs:?}"
    );
}

#[test]
fn native_expiry_debug_redaction() {
    let marker = "SECRET-CLAIM-VALUE";
    let rds = rds_token(marker, marker);
    let (rds_result, rds_logs) = capture(|| parse_token_expiration(TokenAuthKind::Rds, &rds));
    assert_eq!(rds_result.unwrap(), None);
    assert!(
        rds_logs
            .iter()
            .any(|log| log.contains("Failed to parse RDS IAM token expiration:"))
    );
    assert!(rds_logs.iter().all(|log| !log.contains(marker)));

    let azure = unpadded_claims_token(&format!("{{\"exp\":\"{marker}\"}}"));
    let (azure_result, azure_logs) =
        capture(|| parse_token_expiration(TokenAuthKind::Azure, &azure));
    assert_eq!(azure_result.unwrap(), None);
    assert!(
        azure_logs
            .iter()
            .any(|log| log.contains("Failed to parse Azure Entra token expiration:"))
    );
    assert!(azure_logs.iter().all(|log| !log.contains(marker)));
}
