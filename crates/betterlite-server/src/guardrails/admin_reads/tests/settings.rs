fn assert_strings(value: &JsonValue, expected: &[&str]) {
    assert_eq!(strings(value), expected, "the list is {value}");
}

#[tokio::test]
async fn settings_lists_every_entity_in_declaration_order() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, SETTINGS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    let entities = field(&parsed, "supported_entities");
    let names = strings(entities);
    assert_eq!(names.len(), 79);
    assert_eq!(names.first().map(String::as_str), Some("CREDIT_CARD"));
    assert_eq!(names.get(1).map(String::as_str), Some("CRYPTO"));
    assert_eq!(names.get(2).map(String::as_str), Some("DATE_TIME"));
    assert_eq!(names.get(19).map(String::as_str), Some("US_MBI"));
    assert_eq!(names.get(38).map(String::as_str), Some("AU_ABN"));
    assert!(
        names.get(39).is_some_and(|name| name.starts_with("AU_A")),
        "an Australian entity keeps its slot"
    );
    assert_eq!(names.get(40).map(String::as_str), Some("AU_TFN"));
    assert_eq!(names.get(72).map(String::as_str), Some("TR_LICENSE_PLATE"));
    assert!(
        names.get(73).is_some_and(|name| name.starts_with("NG_")),
        "a Nigerian entity keeps its slot"
    );
    assert_eq!(
        names.get(74).map(String::as_str),
        Some("NG_VEHICLE_REGISTRATION")
    );
    assert_eq!(names.last().map(String::as_str), Some("ZA_ID_NUMBER"));
    let unique: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(unique.len(), 79);
}

#[tokio::test]
async fn settings_lists_the_actions_and_modes() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, SETTINGS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    assert_strings(field(&parsed, "supported_actions"), &["BLOCK", "MASK"]);
    assert_strings(
        field(&parsed, "supported_modes"),
        &[
            "pre_call",
            "post_call",
            "during_call",
            "logging_only",
            "pre_mcp_call",
            "during_mcp_call",
            "post_mcp_call",
            "realtime_input_transcription",
        ],
    );
}

#[tokio::test]
async fn settings_lists_the_pii_entity_categories() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, SETTINGS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    let supported: BTreeSet<String> = strings(field(&parsed, "supported_entities"))
        .into_iter()
        .collect();
    let categories = field(&parsed, "pii_entity_categories")
        .as_array()
        .expect("the category list");
    assert_eq!(categories.len(), 20);
    let first = categories.first().expect("a first category");
    assert_eq!(text(field(first, "category")), "General");
    assert_strings(
        field(first, "entities"),
        &[
            "DATE_TIME",
            "EMAIL_ADDRESS",
            "IP_ADDRESS",
            "NRP",
            "LOCATION",
            "PERSON",
            "PHONE_NUMBER",
            "MEDICAL_LICENSE",
            "URL",
            "MAC_ADDRESS",
            "UUID",
        ],
    );
    let listed: BTreeSet<String> = categories
        .iter()
        .flat_map(|category| strings(field(category, "entities")))
        .collect();
    assert!(
        listed.is_subset(&supported),
        "every listed entity is a supported entity"
    );
}

#[rstest]
#[case::bedrock("bedrock", &["pre_call", "post_call", "during_call", "pre_mcp_call", "during_mcp_call"])]
#[case::hide_secrets("hide-secrets", &["pre_call"])]
#[case::guardrails_ai("guardrails_ai", &["post_call", "pre_call", "logging_only"])]
#[case::custom_code("custom_code", &["pre_call", "during_call", "post_call", "pre_mcp_call", "during_mcp_call", "logging_only"])]
#[case::litellm_content_filter("litellm_content_filter", &[
    "pre_call",
    "post_call",
    "during_call",
    "realtime_input_transcription",
    "pre_mcp_call",
    "post_mcp_call",
])]
#[case::prompt_shield("azure/prompt_shield", &["pre_call", "during_call"])]
#[tokio::test]
async fn settings_maps_every_provider_mode(#[case] provider: &str, #[case] hooks: &[&str]) {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, SETTINGS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    let providers = field(&parsed, "supported_modes_by_provider");
    assert_strings(field(providers, provider), hooks);
}

#[tokio::test]
async fn settings_lists_only_supported_hooks_for_each_provider() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, SETTINGS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    let modes: BTreeSet<String> = strings(field(&parsed, "supported_modes"))
        .into_iter()
        .collect();
    let providers = field(&parsed, "supported_modes_by_provider")
        .as_object()
        .expect("the provider map");
    assert_eq!(providers.len(), 54);
    let listed: BTreeSet<String> = providers.values().flat_map(strings).collect();
    assert!(listed.is_subset(&modes), "every hook is a supported mode");
    assert!(
        providers
            .values()
            .all(|hooks| hooks.as_array().is_some_and(|list| !list.is_empty())),
        "every provider has a hook list"
    );
}

#[tokio::test]
async fn settings_content_filter_settings_match_the_catalog() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, SETTINGS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    let settings = field(&parsed, "content_filter_settings");
    let patterns = field(settings, "prebuilt_patterns")
        .as_array()
        .expect("the pattern list");
    assert_eq!(patterns.len(), 82);
    let first_pattern = patterns.first().expect("a first pattern");
    assert_eq!(text(field(first_pattern, "name")), "us_ssn");
    assert_eq!(
        text(field(first_pattern, "display_name")),
        "SSN (Social Security Number)"
    );
    assert_eq!(text(field(first_pattern, "category")), "PII Patterns");
    assert_strings(field(settings, "supported_actions"), &["BLOCK", "MASK"]);
}

#[tokio::test]
async fn settings_lists_the_content_filter_pattern_categories() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, SETTINGS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    let settings = field(&parsed, "content_filter_settings");
    assert_strings(
        field(settings, "pattern_categories"),
        &[
            "PII Patterns",
            "Payment Card Patterns",
            "Credential Patterns",
            "Network Patterns",
            "Protected Class - Fair Lending",
            "Dangerous Content",
            "Dangerous Content - High Risk",
            "Dangerous Content - Crisis",
            "Dangerous Content - Illegal",
            "Brazilian PII Patterns",
            "EU PII Patterns",
            "Aviation PII Patterns",
            "UAE PII Patterns",
            "Singapore PII Patterns",
            "Canadian PII Patterns",
            "Canadian Institutional Identifiers (FIPPA)",
        ],
    );
    let categories = field(settings, "content_categories")
        .as_array()
        .expect("the content category list");
    assert_eq!(categories.len(), 32);
    let first_category = categories.first().expect("a first content category");
    assert_eq!(text(field(first_category, "name")), "age_discrimination");
    assert_eq!(text(field(first_category, "default_action")), "BLOCK");
}

#[tokio::test]
async fn settings_lists_the_decision_model_providers() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, SETTINGS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    assert_strings(
        field(&parse(&body), "decision_model_providers"),
        &[
            "openai",
            "openrouter",
            "azure_ai",
            "perplexity",
            "typesafe",
            "strands_decider",
            "cloudflare",
            "hosted_vllm",
        ],
    );
}

#[tokio::test]
async fn settings_requires_a_valid_key() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, SETTINGS_PATH, None).await;
    assert_unauthorized(status, &body);
}
