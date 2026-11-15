#[test]
fn stable_id_matches_litellm_for_a_fresh_name() {
    let id = stable_guardrail_id("pii-guard", &|_candidate| false);
    assert_eq!(id, PII_GUARD_ID);
}

#[test]
fn stable_id_uses_the_first_suffix_when_the_name_id_is_taken() {
    let taken = |candidate: &str| candidate == PII_GUARD_ID;
    assert_eq!(stable_guardrail_id("pii-guard", &taken), PII_GUARD_ID_1);
}

#[test]
fn stable_id_uses_the_second_suffix_when_two_ids_are_taken() {
    let taken = |candidate: &str| candidate == PII_GUARD_ID || candidate == PII_GUARD_ID_1;
    assert_eq!(stable_guardrail_id("pii-guard", &taken), PII_GUARD_ID_2);
}

#[test]
fn stable_id_returns_the_name_id_when_only_a_suffix_is_taken() {
    let taken = |candidate: &str| candidate == PII_GUARD_ID_1;
    assert_eq!(stable_guardrail_id("pii-guard", &taken), PII_GUARD_ID);
}

#[test]
fn stable_id_hashes_non_ascii_names_as_utf8() {
    let id = stable_guardrail_id("garde-écran", &|_candidate| false);
    assert_eq!(id, "9423e2e1-f7a1-5db9-9199-54fabb164514");
}
