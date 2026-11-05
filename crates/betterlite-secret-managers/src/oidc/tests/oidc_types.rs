use crate::oidc::token_ttl::{EXPIRY_MARGIN_SECONDS, OidcTokenClaims};

#[test]
fn expiry_margin_is_sixty_seconds() {
    assert_eq!(EXPIRY_MARGIN_SECONDS, 60);
}

#[rstest::rstest]
#[case::absent("{}", None)]
#[case::null(r#"{"exp": null}"#, None)]
#[case::integer(r#"{"exp": 1700000000}"#, Some(1_700_000_000.0_f64))]
#[case::fraction(r#"{"exp": 1700000000.5}"#, Some(1_700_000_000.5_f64))]
#[case::unknown_fields_ignored(r#"{"sub": "x", "exp": 5, "aud": ["a"]}"#, Some(5.0_f64))]
fn claims_exp_is_an_optional_float(#[case] json: &str, #[case] expected: Option<f64>) {
    let claims: OidcTokenClaims = serde_json::from_str(json).unwrap();
    assert_eq!(claims.exp, expected);
}

#[rstest::rstest]
#[case::string(r#"{"exp": "1700000000"}"#)]
#[case::array("[]")]
fn claims_reject_a_wrong_type(#[case] json: &str) {
    serde_json::from_str::<OidcTokenClaims>(json).unwrap_err();
}
