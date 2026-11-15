#[test]
fn build_bedrock_sets_its_hooks_and_settings() {
    let callback = build_one(
        "bedrock",
        r#"{"guardrail":"bedrock","mode":"pre_call","mask_request_content":true,"only_scan_new_messages":true,"default_on":true,"timeout":2.5}"#,
        "bedrock-guard",
    );
    let settings = &callback.base.settings;
    assert_eq!(settings.guardrail_name.as_deref(), Some("bedrock-guard"));
    assert_eq!(
        settings.supported_event_hooks,
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::DuringCall,
            GuardrailEventHooks::PreMcpCall,
            GuardrailEventHooks::DuringMcpCall,
        ])
    );
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
    assert!(settings.default_on);
    assert!(settings.mask_request_content);
    assert!(!settings.mask_response_content);
    assert!(settings.only_scan_new_messages);
    assert_eq!(settings.timeout, Some(2.5_f64));
    assert!(matches!(&callback.spec, CallbackSpec::Bedrock(_)));
}

#[test]
fn build_lakera_sets_its_two_hooks() {
    let callback = build_one(
        "lakera",
        r#"{"guardrail":"lakera","mode":"pre_call","default_on":true,"timeout":1.5}"#,
        "lakera-guard",
    );
    let settings = &callback.base.settings;
    assert_eq!(
        settings.supported_event_hooks,
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::DuringCall,
        ])
    );
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
    assert!(settings.default_on);
    assert_eq!(settings.timeout, Some(1.5_f64));
    assert!(matches!(&callback.spec, CallbackSpec::Lakera(_)));
}

#[test]
fn build_lakera_v2_sets_its_three_hooks() {
    let callback = build_one(
        "lakera_v2",
        r#"{"guardrail":"lakera_v2","mode":"during_call","default_on":true,"timeout":2.5}"#,
        "lakera-v2-guard",
    );
    let settings = &callback.base.settings;
    assert_eq!(
        settings.supported_event_hooks,
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::DuringCall,
            GuardrailEventHooks::PostCall,
        ])
    );
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::One("during_call".to_owned()))
    );
    assert!(settings.default_on);
    assert_eq!(settings.timeout, Some(2.5_f64));
    assert!(matches!(&callback.spec, CallbackSpec::LakeraV2(_)));
}

#[test]
fn build_tool_permission_sets_its_hooks_and_template() {
    let callback = build_one(
        "tool_permission",
        r#"{"guardrail":"tool_permission","mode":"post_call","default_on":true,"timeout":4.0,"violation_message_template":"Blocked tool"}"#,
        "tool-guard",
    );
    let settings = &callback.base.settings;
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::One("post_call".to_owned()))
    );
    assert!(settings.default_on);
    assert_eq!(
        settings.supported_event_hooks,
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
        ])
    );
    assert_eq!(
        settings.violation_message_template.as_deref(),
        Some("Blocked tool")
    );
    assert_eq!(settings.timeout, None);
    assert!(matches!(&callback.spec, CallbackSpec::ToolPermission(_)));
}

#[test]
fn build_presidio_orders_scan_restore_and_mask() {
    let callbacks = factories()
        .build("presidio", &params(PRESIDIO_PARAMS), "pii-guard", true)
        .unwrap();
    assert_eq!(callbacks.len(), 3);
    assert_eq!(
        callbacks[0].base.settings.supported_event_hooks,
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::DuringCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::LoggingOnly,
            GuardrailEventHooks::PreMcpCall,
            GuardrailEventHooks::PostMcpCall,
        ])
    );
    assert!(matches!(
        &callbacks[0].spec,
        CallbackSpec::Presidio(args) if args.role == PresidioRole::Scan && !args.apply_to_output
    ));
    assert!(matches!(
        &callbacks[1].spec,
        CallbackSpec::Presidio(args) if args.role == PresidioRole::Restore
    ));
    assert!(matches!(
        &callbacks[2].spec,
        CallbackSpec::Presidio(args) if args.role == PresidioRole::Scan && args.apply_to_output
    ));
}

#[test]
fn build_presidio_sets_the_mask_flag_and_modes_per_callback() {
    let callbacks = factories()
        .build("presidio", &params(PRESIDIO_PARAMS), "pii-guard", true)
        .unwrap();
    assert!(!callbacks[0].base.settings.mask_response_content);
    assert!(callbacks[2].base.settings.mask_response_content);
    assert_eq!(
        callbacks[1].base.settings.event_hook,
        Some(LitellmParamsMode::Many(vec!["post_call".to_owned()]))
    );
    assert!(
        callbacks
            .iter()
            .all(|callback| callback.base.settings.guardrail_name.as_deref() == Some("pii-guard"))
    );
}

#[test]
fn build_presidio_copies_the_common_settings_to_every_callback() {
    let callbacks = factories()
        .build(
            "presidio",
            &params(
                r#"{"guardrail":"presidio","mode":"pre_call","output_parse_pii":true,"default_on":true,"timeout":3.5}"#,
            ),
            "pii-guard",
            true,
        )
        .unwrap();
    assert!(callbacks.iter().all(|callback| {
        callback.base.settings.default_on && callback.base.settings.timeout == Some(3.5_f64)
    }));
    assert_eq!(
        callbacks[0].base.settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
}

#[test]
fn build_tool_policy_uses_the_caller_ports() {
    let callback = build_one(
        "tool_policy",
        r#"{"guardrail":"tool_policy","mode":"pre_call"}"#,
        "policy-guard",
    );
    let settings = &callback.base.settings;
    assert_eq!(settings.guardrail_name.as_deref(), Some("policy-guard"));
    assert_eq!(
        settings.supported_event_hooks,
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::DuringCall,
        ])
    );
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
    assert!(matches!(&callback.spec, CallbackSpec::ToolPolicy(_)));
}

#[test]
fn build_hide_secrets_always_reports_the_missing_package() {
    let error = factories()
        .build(
            "hide-secrets",
            &params(r#"{"guardrail":"hide-secrets","mode":"pre_call"}"#),
            "secret-guard",
            true,
        )
        .err()
        .unwrap();
    assert!(matches!(
        error,
        RegistryError::Enterprise(InitializerError::MissingEnterprisePackage)
    ));
    assert!(error.to_string().contains("litellm-enterprise"));
}

#[test]
fn build_rejects_kinds_without_a_builder() {
    let grey = factories()
        .build(
            "grayswan",
            &params(r#"{"guardrail":"grayswan","mode":"pre_call"}"#),
            "grey",
            true,
        )
        .err()
        .unwrap();
    assert_eq!(grey.to_string(), "Unsupported guardrail: grayswan");
    let judge = factories()
        .build(
            "llm_as_a_judge",
            &params(r#"{"guardrail":"llm_as_a_judge","mode":"pre_call"}"#),
            "judge",
            true,
        )
        .err()
        .unwrap();
    assert_eq!(judge.to_string(), "Unsupported guardrail: llm_as_a_judge");
    let custom = factories()
        .build(
            "my_module.MyGuardrail",
            &params(r#"{"guardrail":"my_module.MyGuardrail","mode":"pre_call"}"#),
            "custom",
            true,
        )
        .err()
        .unwrap();
    assert_eq!(
        custom.to_string(),
        "Unsupported guardrail: my_module.MyGuardrail"
    );
}

#[test]
fn build_requires_a_guardrail_type() {
    let error = factories()
        .build(
            "",
            &params(r#"{"guardrail":"","mode":"pre_call"}"#),
            "empty",
            true,
        )
        .err()
        .unwrap();
    assert!(matches!(error, RegistryError::MissingGuardrailType));
    assert_eq!(error.to_string(), "guardrail_type is required");
}

#[test]
fn build_rejects_a_mode_the_kind_does_not_support() {
    let error = factories()
        .build(
            "lakera",
            &params(r#"{"guardrail":"lakera","mode":"post_call"}"#),
            "lakera-guard",
            true,
        )
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "Event hook post_call is not in the supported event hooks [pre_call, during_call]"
    );
}

#[test]
fn build_keeps_an_unsupported_mode_when_modes_are_lenient() {
    let callbacks = factories()
        .build(
            "lakera",
            &params(r#"{"guardrail":"lakera","mode":"post_call"}"#),
            "lakera-guard",
            false,
        )
        .unwrap();
    assert_eq!(callbacks.len(), 1);
    assert_eq!(
        callbacks[0].base.settings.event_hook,
        Some(LitellmParamsMode::One("post_call".to_owned()))
    );
}

#[test]
fn build_built_kinds_are_not_compression() {
    let cases = [
        ("bedrock", BEDROCK_PARAMS),
        ("lakera", r#"{"guardrail":"lakera","mode":"pre_call"}"#),
        (
            "lakera_v2",
            r#"{"guardrail":"lakera_v2","mode":"pre_call"}"#,
        ),
        ("presidio", r#"{"guardrail":"presidio","mode":"pre_call"}"#),
        (
            "tool_permission",
            r#"{"guardrail":"tool_permission","mode":"pre_call"}"#,
        ),
    ];
    let plain = cases.iter().all(|(kind, params_json)| {
        let built = factories()
            .build(kind, &params(params_json), "guard", true)
            .unwrap();
        built.iter().all(|callback| !callback.is_compression())
    });
    assert!(plain);
}

#[test]
fn build_compression_spec_is_recognized() {
    let mut callbacks = factories()
        .build("bedrock", &params(BEDROCK_PARAMS), "compress", true)
        .unwrap();
    assert!(!callbacks[0].is_compression());
    super::handler::mark_compression(&mut callbacks, "bedrock");
    assert!(!callbacks[0].is_compression());
    super::handler::mark_compression(&mut callbacks, "headroom");
    assert!(callbacks[0].is_compression());
    assert!(matches!(
        &callbacks[0].spec,
        CallbackSpec::Compression(CompressionKind::Headroom)
    ));
}

#[test]
fn build_compression_kind_matches_only_the_three_providers() {
    assert_eq!(
        compression_kind("headroom"),
        Some(CompressionKind::Headroom)
    );
    assert_eq!(
        compression_kind("compresr"),
        Some(CompressionKind::Compresr)
    );
    assert_eq!(
        compression_kind("typesafe"),
        Some(CompressionKind::Typesafe)
    );
    assert_eq!(compression_kind("bedrock"), None);
    assert_eq!(compression_kind(""), None);
}
