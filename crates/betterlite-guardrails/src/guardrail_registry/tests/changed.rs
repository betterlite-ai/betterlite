#[test]
fn changed_reports_a_new_guardrail() {
    let incoming = guardrail(None, "pii-guard", BEDROCK_PARAMS);
    assert!(params_changed(None, &incoming).unwrap());
}

#[test]
fn changed_reports_a_new_name() {
    let existing = guardrail(None, "pii-guard", BEDROCK_PARAMS);
    let renamed = guardrail(None, "other-guard", BEDROCK_PARAMS);
    assert!(params_changed(Some(&existing), &renamed).unwrap());
}

#[test]
fn changed_reports_a_different_param() {
    let existing = guardrail(None, "pii-guard", BEDROCK_PARAMS);
    let enabled = guardrail(
        None,
        "pii-guard",
        r#"{"guardrail":"bedrock","mode":"pre_call","default_on":true}"#,
    );
    assert!(params_changed(Some(&existing), &enabled).unwrap());
}

#[test]
fn changed_reports_an_added_param_key() {
    let existing = guardrail(None, "pii-guard", BEDROCK_PARAMS);
    let with_budget = guardrail(
        None,
        "pii-guard",
        r#"{"guardrail":"bedrock","mode":"pre_call","chunk_budget_chars":10}"#,
    );
    assert!(params_changed(Some(&existing), &with_budget).unwrap());
    assert!(params_changed(Some(&with_budget), &existing).unwrap());
}

#[test]
fn changed_missing_key_reads_as_null() {
    let existing = guardrail(
        None,
        "pii-guard",
        r#"{"guardrail":"bedrock","mode":"pre_call","chunk_budget_chars":null}"#,
    );
    let incoming = guardrail(None, "pii-guard", BEDROCK_PARAMS);
    assert!(!params_changed(Some(&existing), &incoming).unwrap());
}

#[test]
fn changed_equal_params_are_unchanged() {
    let existing = guardrail(None, "pii-guard", BEDROCK_PARAMS);
    let incoming = existing.clone();
    assert!(!params_changed(Some(&existing), &incoming).unwrap());
}
