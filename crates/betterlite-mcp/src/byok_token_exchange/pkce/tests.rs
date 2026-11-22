#![cfg(test)]

use rstest::rstest;

use super::verify_pkce;

// the verifier and challenge of RFC 7636 appendix B
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

#[rstest]
#[case::rfc7636_example(VERIFIER, CHALLENGE, true)]
#[case::wrong_verifier("not-the-verifier", CHALLENGE, false)]
#[case::padded_challenge(VERIFIER, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM=", false)]
#[case::empty_verifier("", "47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU", true)]
#[case::empty_verifier_wrong_challenge("", CHALLENGE, false)]
#[case::empty_challenge(VERIFIER, "", false)]
#[case::non_ascii_verifier(
    "\u{fc}\u{f1}\u{ef}\u{e7}\u{f8}d\u{e9}",
    "CRq1B87KKMcG1TcI6MGzKFE7NtDLjfpfofPinwjxMOs",
    true
)]
#[case::non_ascii_verifier_wrong_challenge(
    "\u{fc}\u{f1}\u{ef}\u{e7}\u{f8}d\u{e9}",
    CHALLENGE,
    false
)]
#[case::challenge_is_case_sensitive(VERIFIER, "e9melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM", false)]
fn verify_pkce_matches_litellm(
    #[case] code_verifier: &str,
    #[case] code_challenge: &str,
    #[case] expected: bool,
) {
    assert_eq!(verify_pkce(code_verifier, code_challenge), expected);
}

#[test]
fn a_prefix_of_the_challenge_does_not_verify() {
    let shorter = CHALLENGE
        .get(..CHALLENGE.len().saturating_sub(1))
        .unwrap_or_default();
    assert!(!verify_pkce(VERIFIER, shorter));
}
