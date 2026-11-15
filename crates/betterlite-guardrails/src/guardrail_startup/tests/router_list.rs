#[test]
fn router_list_has_one_entry_per_guardrail_in_order_with_the_four_params() {
    let (captured, _guard) = capture();
    let params = r#"{"guardrail":"bedrock","mode":["pre_call","post_call"],"api_key":"the-key","api_base":"https://guard.example"}"#;
    let held = vec![
        guardrail(Some("one"), "first", params),
        guardrail(None, "second", BEDROCK_POST),
    ];
    let list = router_guardrail_list(&held);
    assert_eq!(list.len(), 2);
    let first = &list[0];
    assert_eq!(first.guardrail_name, "first");
    assert_eq!(first.id.as_deref(), Some("one"));
    assert_eq!(first.guardrail, "bedrock");
    assert_eq!(
        first.mode,
        LitellmParamsMode::Many(vec!["pre_call".to_owned(), "post_call".to_owned()])
    );
    assert_eq!(first.api_key.as_ref().unwrap().expose_secret(), "the-key");
    assert_eq!(first.api_base.as_deref(), Some("https://guard.example"));
    let second = &list[1];
    assert_eq!(second.guardrail_name, "second");
    assert_eq!(second.id, None);
    assert_eq!(second.mode, LitellmParamsMode::One("post_call".to_owned()));
    assert!(second.api_key.is_none());
    assert!(second.api_base.is_none());
    assert!(logged(&captured).contains("Populated router guardrail_list with 2 guardrails"));
}

#[test]
fn router_list_debug_never_prints_the_api_key() {
    let params = r#"{"guardrail":"bedrock","mode":"pre_call","api_key":"the-key"}"#;
    let list = router_guardrail_list(&[guardrail(Some("one"), "first", params)]);
    assert!(!format!("{list:?}").contains("the-key"));
}

#[test]
fn router_list_of_no_guardrails_is_empty() {
    let (captured, _guard) = capture();
    assert!(router_guardrail_list(&[]).is_empty());
    assert!(logged(&captured).contains("Populated router guardrail_list with 0 guardrails"));
}
