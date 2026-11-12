use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::jwt_claims::decode::{
    DecodeError, MAX_CLAIM_BYTES, MAX_CLAIM_DEPTH, decode_unverified_claims, unverified_claims,
};

use super::support::{document, segment, token};

#[test]
fn decodes_a_base64url_payload_segment() {
    let decoded = decode_unverified_claims(segment(r#"{"sub":"user-1"}"#).as_bytes()).unwrap();
    assert_eq!(decoded, document(r#"{"sub":"user-1"}"#));
}

#[test]
fn accepts_padding_pyjwt_accepts() {
    let double_padded = "eyJhIjoxfQ==";
    assert_eq!(
        decode_unverified_claims(double_padded.as_bytes()).unwrap(),
        document(r#"{"a":1}"#)
    );
    let unpadded = "eyJhIjoxfQ";
    assert_eq!(
        decode_unverified_claims(unpadded.as_bytes()).unwrap(),
        document(r#"{"a":1}"#)
    );
}

#[test]
fn rejects_segments_pyjwt_rejects() {
    assert!(matches!(
        decode_unverified_claims(b"eyJhIjoxfQ="),
        Err(DecodeError::Padding)
    ));
    assert!(matches!(
        decode_unverified_claims(b"eyJhIjoxfQ==="),
        Err(DecodeError::Padding)
    ));
    assert!(matches!(
        decode_unverified_claims(b"a"),
        Err(DecodeError::Padding)
    ));
    assert!(matches!(
        decode_unverified_claims(b"@@@"),
        Err(DecodeError::Base64(_))
    ));
    assert!(matches!(
        decode_unverified_claims(b"eyJhIjoxfR"),
        Err(DecodeError::Base64(_))
    ));
}

#[test]
fn rejects_payloads_that_are_not_objects() {
    assert!(matches!(
        decode_unverified_claims(segment("[1,2]").as_bytes()),
        Err(DecodeError::NotAnObject)
    ));
    assert!(matches!(
        decode_unverified_claims(segment("not json").as_bytes()),
        Err(DecodeError::Json(_))
    ));
    assert!(matches!(
        decode_unverified_claims(b""),
        Err(DecodeError::Json(_))
    ));
}

#[test]
fn duplicate_fields_keep_the_last_value() {
    let decoded = decode_unverified_claims(segment(r#"{"a":1,"a":2}"#).as_bytes()).unwrap();
    assert_eq!(decoded, document(r#"{"a":2}"#));
}

fn nested_json(depth: usize) -> String {
    format!("{}null{}", "{\"a\":".repeat(depth), "}".repeat(depth))
}

#[test]
fn native_new_otherwise_unbounded_byte_decoders_reject_over_1048576_bytes_or_jso() {
    let accepted = nested_json(MAX_CLAIM_DEPTH);
    drop(decode_unverified_claims(segment(&accepted).as_bytes()).unwrap());
    let rejected = nested_json(MAX_CLAIM_DEPTH.saturating_add(1));
    assert!(matches!(
        decode_unverified_claims(segment(&rejected).as_bytes()),
        Err(DecodeError::TooDeep { limit }) if limit == MAX_CLAIM_DEPTH
    ));
    let at_limit = vec![b'A'; MAX_CLAIM_BYTES];
    assert!(matches!(
        decode_unverified_claims(&at_limit),
        Err(DecodeError::Json(_))
    ));
    let oversized = vec![b'A'; MAX_CLAIM_BYTES.saturating_add(1)];
    assert!(matches!(
        decode_unverified_claims(&oversized),
        Err(DecodeError::TooLarge { bytes, limit })
            if bytes == MAX_CLAIM_BYTES.saturating_add(1) && limit == MAX_CLAIM_BYTES
    ));
}

#[test]
fn unverified_claims_requires_three_segments() {
    assert_eq!(unverified_claims("a.b"), None);
    assert_eq!(unverified_claims("a.b.c.d"), None);
    assert_eq!(unverified_claims(""), None);
}

#[test]
fn unverified_claims_reads_the_payload_object() {
    let valid = token(r#"{"alg":"RS256"}"#, r#"{"sub":"user-1"}"#, "signature");
    assert_eq!(
        unverified_claims(&valid),
        Some(document(r#"{"sub":"user-1"}"#))
    );
    let empty_signature = format!(
        "{}.{}.",
        segment(r#"{"alg":"RS256"}"#),
        segment(r#"{"sub":"user-1"}"#)
    );
    assert_eq!(
        unverified_claims(&empty_signature),
        Some(document(r#"{"sub":"user-1"}"#))
    );
}

#[rstest]
#[case::header_not_object("[]", r#"{"sub":"user-1"}"#)]
#[case::invalid_json_header("not json", r#"{"sub":"user-1"}"#)]
#[case::kid_not_string(r#"{"alg":"RS256","kid":5}"#, r#"{"sub":"user-1"}"#)]
#[case::unsupported_crit(r#"{"alg":"RS256","crit":["exp"],"exp":1}"#, r#"{"sub":"user-1"}"#)]
#[case::empty_crit(r#"{"alg":"RS256","crit":[]}"#, r#"{"sub":"user-1"}"#)]
#[case::detached_payload(r#"{"alg":"RS256","b64":false}"#, r#"{"sub":"user-1"}"#)]
#[case::payload_not_object(r#"{"alg":"RS256"}"#, "[1,2]")]
#[case::malformed_payload(r#"{"alg":"RS256"}"#, "not json")]
fn unverified_claims_rejects_unusable_tokens(#[case] header: &str, #[case] payload: &str) {
    let rejected = token(header, payload, "signature");
    assert_eq!(unverified_claims(&rejected), None);
}

#[test]
fn unverified_claims_accepts_a_supported_critical_extension() {
    let accepted = token(
        r#"{"alg":"RS256","crit":["b64"],"b64":true}"#,
        r#"{"sub":"user-1"}"#,
        "signature",
    );
    assert!(unverified_claims(&accepted).is_some());
}

#[test]
fn unverified_claims_validates_the_signature_segment() {
    let rejected = format!(
        "{}.{}.!!!",
        segment(r#"{"alg":"RS256"}"#),
        segment(r#"{"sub":"user-1"}"#)
    );
    assert_eq!(unverified_claims(&rejected), None);
}

#[rstest]
fn replays_the_jwt_claims_corpus(#[files("fuzz/corpus/jwt_claims/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_unverified_claims(&bytes));
}
