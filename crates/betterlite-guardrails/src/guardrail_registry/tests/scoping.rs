#[test]
fn scoping_copies_the_three_values() {
    let parsed = params(
        r#"{"guardrail":"lakera","mode":"pre_call","skip_system_message_in_guardrail":true,"skip_tool_message_in_guardrail":false,"scan_only_tool_results":false}"#,
    );
    let mut callback = factories()
        .build("lakera", &parsed, "lakera-guard", true)
        .unwrap()
        .remove(0);
    configure_scoping(&mut callback, "lakera-guard", &parsed, true).unwrap();
    assert_eq!(
        callback.scoping,
        CallbackScoping {
            skip_system_message: Some(true),
            skip_tool_message: Some(false),
            scan_only_tool_results: Some(false),
        }
    );
}

#[test]
fn scoping_rejects_scan_only_for_latest_role_bedrock() {
    let parsed = params(
        r#"{"guardrail":"bedrock","mode":"pre_call","experimental_use_latest_role_message_only":true,"scan_only_tool_results":true}"#,
    );
    let mut callback = factories()
        .build("bedrock", &parsed, "latest-guard", true)
        .unwrap()
        .remove(0);
    let error = configure_scoping(&mut callback, "latest-guard", &parsed, false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Guardrail latest-guard: scan_only_tool_results is enabled, but this guardrail's role filtering never scans tool results, so no request content would ever be scanned. Remove scan_only_tool_results or the guardrail's role-filtering option."
    );
}

#[test]
fn scoping_allows_scan_only_for_bedrock_without_latest_role() {
    let parsed =
        params(r#"{"guardrail":"bedrock","mode":"pre_call","scan_only_tool_results":true}"#);
    let mut callback = factories()
        .build("bedrock", &parsed, "bedrock-guard", true)
        .unwrap()
        .remove(0);
    configure_scoping(&mut callback, "bedrock-guard", &parsed, false).unwrap();
    assert_eq!(callback.scoping.scan_only_tool_results, Some(true));
}

#[test]
fn scoping_rejects_scan_only_with_the_callbacks_own_skip_tool() {
    let parsed = params(
        r#"{"guardrail":"lakera","mode":"pre_call","scan_only_tool_results":true,"skip_tool_message_in_guardrail":true}"#,
    );
    let mut callback = factories()
        .build("lakera", &parsed, "skip-guard", true)
        .unwrap()
        .remove(0);
    let error = configure_scoping(&mut callback, "skip-guard", &parsed, false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Guardrail skip-guard: scan_only_tool_results and skip_tool_message_in_guardrail are enabled together, which excludes every message from scanning, so no request content would ever be scanned. Remove one of the two."
    );
}

#[test]
fn scoping_falls_back_to_the_global_skip_tool() {
    let parsed =
        params(r#"{"guardrail":"lakera","mode":"pre_call","scan_only_tool_results":true}"#);
    let mut callback = factories()
        .build("lakera", &parsed, "global-guard", true)
        .unwrap()
        .remove(0);
    let error = configure_scoping(&mut callback, "global-guard", &parsed, true).unwrap_err();
    assert!(matches!(
        error,
        RegistryError::ScanOnlyWithSkipTool { guardrail_name } if guardrail_name == "global-guard"
    ));
}

#[test]
fn scoping_prefers_the_callbacks_own_skip_tool_over_the_global() {
    let parsed = params(
        r#"{"guardrail":"lakera","mode":"pre_call","scan_only_tool_results":true,"skip_tool_message_in_guardrail":false}"#,
    );
    let mut callback = factories()
        .build("lakera", &parsed, "own-guard", true)
        .unwrap()
        .remove(0);
    configure_scoping(&mut callback, "own-guard", &parsed, true).unwrap();
}

#[test]
fn scoping_keeps_the_constructor_defaults_when_params_leave_them_out() {
    let parsed = params(r#"{"guardrail":"lakera","mode":"pre_call"}"#);
    let mut callback = factories()
        .build("lakera", &parsed, "default-guard", true)
        .unwrap()
        .remove(0);
    callback.base.settings.run_in_parallel = true;
    callback.base.settings.scan_raw_request = true;
    configure_scoping(&mut callback, "default-guard", &parsed, false).unwrap();
    assert!(callback.base.settings.run_in_parallel);
    assert!(callback.base.settings.scan_raw_request);
}

#[test]
fn scoping_overwrites_the_two_settings_when_the_params_set_them() {
    let parsed = params(
        r#"{"guardrail":"lakera","mode":"pre_call","run_in_parallel":false,"scan_raw_request":false}"#,
    );
    let mut callback = factories()
        .build("lakera", &parsed, "override-guard", true)
        .unwrap()
        .remove(0);
    callback.base.settings.run_in_parallel = true;
    callback.base.settings.scan_raw_request = true;
    configure_scoping(&mut callback, "override-guard", &parsed, false).unwrap();
    assert!(!callback.base.settings.run_in_parallel);
    assert!(!callback.base.settings.scan_raw_request);
}

#[test]
fn scoping_allows_latest_role_bedrock_without_scan_only() {
    let parsed = params(
        r#"{"guardrail":"bedrock","mode":"pre_call","experimental_use_latest_role_message_only":true}"#,
    );
    let mut callback = factories()
        .build("bedrock", &parsed, "latest-guard", true)
        .unwrap()
        .remove(0);
    configure_scoping(&mut callback, "latest-guard", &parsed, false).unwrap();
    assert_eq!(callback.scoping.scan_only_tool_results, None);
}

#[test]
fn scoping_allows_skip_tool_without_scan_only() {
    let parsed =
        params(r#"{"guardrail":"lakera","mode":"pre_call","skip_tool_message_in_guardrail":true}"#);
    let mut callback = factories()
        .build("lakera", &parsed, "skip-guard", true)
        .unwrap()
        .remove(0);
    configure_scoping(&mut callback, "skip-guard", &parsed, true).unwrap();
    assert_eq!(callback.scoping.skip_tool_message, Some(true));
}
