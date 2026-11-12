#[test]
fn env_references_resolve_in_text_keys() {
    let settings = load(
        &minimal("      api_key: os.environ/GUARD_KEY\n      api_base: os.environ/GUARD_BASE\n"),
        &[
            ("GUARD_KEY", "from-env"),
            ("GUARD_BASE", "https://env.test"),
        ],
    );
    let fields = &settings.guardrails[0].litellm_params.base.fields;
    assert_eq!(fields.api_key.as_ref().unwrap().expose_secret(), "from-env");
    assert_eq!(fields.api_base.as_deref(), Some("https://env.test"));
}

const ENTRY_REFERENCES: &str = "guardrails:\n  - guardrail_name: os.environ/ENTRY_NAME\n    guardrail_id: os.environ/ENTRY_ID\n    policy_template: os.environ/ENTRY_TEMPLATE\n    created_at: os.environ/ENTRY_CREATED\n    updated_at: os.environ/ENTRY_UPDATED\n    litellm_params: {guardrail: custom, mode: pre_call}\n";

#[test]
fn env_entry_text_fields_resolve() {
    let settings = load(
        ENTRY_REFERENCES,
        &[
            ("ENTRY_NAME", "from-env"),
            ("ENTRY_ID", "id-9"),
            ("ENTRY_TEMPLATE", "tpl-9"),
            ("ENTRY_CREATED", "2025-01-01T00:00:00Z"),
            ("ENTRY_UPDATED", "2025-02-03T04:05:06Z"),
        ],
    );
    let held = &settings.guardrails[0];
    assert_eq!(held.guardrail_name, "from-env");
    assert_eq!(held.guardrail_id.as_deref(), Some("id-9"));
    assert_eq!(held.policy_template.as_deref(), Some("tpl-9"));
    assert_eq!(
        held.created_at.unwrap().to_rfc3339(),
        "2025-01-01T00:00:00+00:00"
    );
    assert_eq!(
        held.updated_at.unwrap().to_rfc3339(),
        "2025-02-03T04:05:06+00:00"
    );
}

#[rstest]
#[case::missing(&[("ENTRY_NAME", "n")])]
#[case::empty(&[("ENTRY_NAME", "n"), ("ENTRY_ID", ""), ("ENTRY_TEMPLATE", ""), ("ENTRY_CREATED", ""), ("ENTRY_UPDATED", "")])]
fn env_entry_missing_or_empty_variable_is_unset(#[case] environment: &[(&str, &str)]) {
    let settings = load(ENTRY_REFERENCES, environment);
    let held = &settings.guardrails[0];
    assert_eq!(held.guardrail_id, None);
    assert_eq!(held.policy_template, None);
    assert_eq!(held.created_at, None);
    assert_eq!(held.updated_at, None);
}

#[rstest]
#[case::name_missing(&[], "os.environ/ENTRY_NAME", "missing field guardrail_name")]
#[case::created_bad(&[("ENTRY_NAME", "n"), ("ENTRY_CREATED", "yesterday")], "n", "invalid field created_at")]
#[case::updated_bad(&[("ENTRY_NAME", "n"), ("ENTRY_UPDATED", "soon")], "n", "invalid field updated_at")]
fn env_entry_bad_value_rejects_only_that_entry(
    #[case] environment: &[(&str, &str)],
    #[case] name: &str,
    #[case] reason: &str,
) {
    let settings = load(ENTRY_REFERENCES, environment);
    assert!(settings.guardrails.is_empty());
    assert_eq!(settings.rejected[0].guardrail_name, name);
    assert_eq!(settings.rejected[0].reason, reason);
}

#[rstest]
#[case::missing(&[])]
#[case::empty(&[("GUARD_BASE", "")])]
fn env_missing_or_empty_variable_is_null(#[case] environment: &[(&str, &str)]) {
    let settings = load(
        &minimal("      api_base: os.environ/GUARD_BASE\n"),
        environment,
    );
    assert!(settings.rejected.is_empty());
    assert_eq!(
        settings.guardrails[0].litellm_params.base.fields.api_base,
        None
    );
}

#[test]
fn env_walk_follows_mappings_and_mappings_inside_lists() {
    let source = minimal(
        "      additional_provider_specific_params:\n        region: os.environ/REGION\n        inner: {key: os.environ/INNER}\n        items: [os.environ/PLAIN, {key: os.environ/ITEM}]\n",
    );
    let settings = load(
        &source,
        &[
            ("REGION", "eu"),
            ("INNER", "deep"),
            ("ITEM", "listed"),
            ("PLAIN", "ignored"),
        ],
    );
    let held = settings.guardrails[0]
        .litellm_params
        .base
        .fields
        .provider_specific_params
        .as_ref()
        .unwrap();
    assert_eq!(held.0["region"], text("eu"));
    assert_eq!(
        held.0["inner"],
        GuardrailValue::Object(BTreeMap::from([("key".to_owned(), text("deep"))]))
    );
    assert_eq!(
        held.0["items"],
        GuardrailValue::Array(vec![
            text("os.environ/PLAIN"),
            GuardrailValue::Object(BTreeMap::from([("key".to_owned(), text("listed"))])),
        ])
    );
}

#[test]
fn env_guardrail_info_is_resolved() {
    let source = "guardrails:\n  - guardrail_name: g\n    guardrail_info: {owner: os.environ/OWNER}\n    litellm_params: {guardrail: custom, mode: pre_call}\n";
    let settings = load(source, &[("OWNER", "platform")]);
    let info = settings.guardrails[0].guardrail_info.as_ref().unwrap();
    assert_eq!(info.0["owner"], text("platform"));
}

#[rstest]
#[case::true_text("true", Some(true))]
#[case::false_text("False", Some(false))]
#[case::empty("", None)]
fn env_bool_keys_use_the_flag_parser(#[case] variable: &str, #[case] expected: Option<bool>) {
    let settings = load(
        &minimal("      inspect_embeddings: os.environ/FLAG\n"),
        &[("FLAG", variable)],
    );
    let parsed = &settings.guardrails[0].litellm_params.base.fields;
    assert_eq!(parsed.inspect_embeddings, expected);
}

#[test]
fn env_bad_bool_text_rejects_only_that_entry() {
    let source = "guardrails:\n  - guardrail_name: bad\n    litellm_params: {guardrail: custom, mode: pre_call, inspect_embeddings: os.environ/FLAG}\n  - guardrail_name: fine\n    litellm_params: {guardrail: custom, mode: pre_call}\n";
    let settings = load(source, &[("FLAG", "perhaps")]);
    assert_eq!(settings.guardrails.len(), 1);
    assert_eq!(settings.guardrails[0].guardrail_name, "fine");
    assert_eq!(settings.rejected[0].guardrail_name, "bad");
    assert!(
        settings.rejected[0].reason.contains("FLAG"),
        "{}",
        settings.rejected[0].reason
    );
    assert!(!settings.rejected[0].reason.contains("perhaps"));
}

#[rstest]
#[case::float("timeout", "2.5")]
#[case::integer("end_session_after_n_fails", "3")]
#[case::positive("max_input_chars", "10")]
fn env_number_keys_use_the_number_parser(#[case] key: &str, #[case] variable: &str) {
    let settings = load(
        &minimal(&format!("      {key}: os.environ/NUMBER\n")),
        &[("NUMBER", variable)],
    );
    assert!(settings.rejected.is_empty(), "{:?}", settings.rejected);
    let written = dump(&settings.guardrails[0].litellm_params.base);
    assert!(matches!(
        written[key],
        GuardrailValue::Decimal(_) | GuardrailValue::SignedInteger(_)
    ));
}

#[rstest]
#[case::timeout_text("timeout", "soon")]
#[case::timeout_not_finite("timeout", "inf")]
#[case::count_text("end_session_after_n_fails", "x")]
#[case::count_float("max_input_chars", "1.5")]
fn env_bad_number_text_names_the_variable(#[case] key: &str, #[case] variable: &str) {
    let settings = load(
        &minimal(&format!("      {key}: os.environ/NUMBER\n")),
        &[("NUMBER", variable)],
    );
    assert!(settings.guardrails.is_empty());
    assert!(
        settings.rejected[0].reason.contains("NUMBER"),
        "{}",
        settings.rejected[0].reason
    );
}

#[test]
fn env_missing_typed_variable_takes_the_default() {
    let settings = load(
        &minimal(
            "      sanitize_error_detail: os.environ/MISSING\n      timeout: os.environ/MISSING\n",
        ),
        &[],
    );
    let fields = &settings.guardrails[0].litellm_params.base.fields;
    assert!(fields.sanitize_error_detail);
    assert_eq!(fields.timeout, None);
}

fn nested(levels: usize, leaf: GuardrailValue) -> GuardrailValue {
    (0..levels).fold(leaf, |inner, _level| {
        GuardrailValue::Object(BTreeMap::from([("k".to_owned(), inner)]))
    })
}

fn leaf_of(mut value: &GuardrailValue) -> &GuardrailValue {
    while let GuardrailValue::Object(map) = value {
        value = &map["k"];
    }
    value
}

#[rstest]
#[case::last_resolved_level(98, text("deep-value"))]
#[case::first_skipped_level(99, text("os.environ/DEEP"))]
fn env_walk_stops_past_the_depth_limit(#[case] levels: usize, #[case] expected: GuardrailValue) {
    let lookup = BTreeMap::from([("DEEP".to_owned(), "deep-value".to_owned())]);
    let input = BTreeMap::from([("extra".to_owned(), nested(levels, text("os.environ/DEEP")))]);
    let resolved = resolve_params(input, &lookup).unwrap();
    assert_eq!(leaf_of(&resolved["extra"]), &expected);
}

#[rstest]
fn replays_the_guardrail_params_corpus(#[files("fuzz/corpus/guardrail_params/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _params = decode_litellm_params(&bytes);
    let _patch = decode_patch_request(&bytes);
    let _apply = decode_apply_request(&bytes);
}
