#[test]
fn as_plain_dict_keeps_a_plain_row() {
    let row = plain_message("user", "hi");
    assert_eq!(as_plain_dict(MessageRow::Plain(&row)), row);
}

#[test]
fn as_plain_dict_dumps_a_typed_message() {
    let typed = message("assistant", "hi");
    assert_eq!(
        as_plain_dict(MessageRow::Typed(&typed)),
        plain_message("assistant", "hi")
    );
}

#[test]
fn as_plain_dict_lets_the_typed_role_win_over_an_extra_role() {
    let typed = GuardrailMessage {
        role: "user".to_owned(),
        extra: BTreeMap::from([("role".to_owned(), text("assistant"))]),
    };
    assert_eq!(
        field(&as_plain_dict(MessageRow::Typed(&typed)), "role"),
        Some(&text("user"))
    );
}

#[test]
fn resolve_threshold_prefers_the_argument() {
    let env = lookup(&[("GRAYSWAN_VIOLATION_THRESHOLD", "0.7")]);
    assert!((resolve_threshold(Some(0.9), &env) - 0.9).abs() < f64::EPSILON);
}

#[test]
fn resolve_threshold_reads_a_trimmed_environment_value() {
    let env = lookup(&[("GRAYSWAN_VIOLATION_THRESHOLD", " 0.25 ")]);
    assert!((resolve_threshold(None, &env) - 0.25).abs() < f64::EPSILON);
}

#[test]
fn resolve_threshold_defaults_when_the_environment_value_is_missing_or_invalid() {
    assert!((resolve_threshold(None, &lookup(&[])) - 0.5).abs() < f64::EPSILON);
    let invalid = lookup(&[("GRAYSWAN_VIOLATION_THRESHOLD", "abc")]);
    assert!((resolve_threshold(None, &invalid) - 0.5).abs() < f64::EPSILON);
    let empty = lookup(&[("GRAYSWAN_VIOLATION_THRESHOLD", "")]);
    assert!((resolve_threshold(None, &empty) - 0.5).abs() < f64::EPSILON);
}

#[test]
fn resolve_threshold_keeps_an_explicit_zero() {
    let env = lookup(&[("GRAYSWAN_VIOLATION_THRESHOLD", "0.7")]);
    assert!(resolve_threshold(Some(0.0), &env).abs() < f64::EPSILON);
    let zero = lookup(&[("GRAYSWAN_VIOLATION_THRESHOLD", "0.0")]);
    assert!(resolve_threshold(None, &zero).abs() < f64::EPSILON);
}

#[test]
fn resolve_reasoning_mode_parses_the_argument_case_insensitively() {
    assert_eq!(
        resolve_reasoning_mode(Some("THINKING"), &lookup(&[])),
        Some(ReasoningMode::Thinking)
    );
}

#[test]
fn resolve_reasoning_mode_falls_back_to_the_environment() {
    let env = lookup(&[("GRAYSWAN_REASONING_MODE", "Hybrid")]);
    assert_eq!(
        resolve_reasoning_mode(None, &env),
        Some(ReasoningMode::Hybrid)
    );
}

#[test]
fn resolve_reasoning_mode_uses_the_environment_for_an_unsupported_argument() {
    let env = lookup(&[("GRAYSWAN_REASONING_MODE", "off")]);
    assert_eq!(
        resolve_reasoning_mode(Some("bogus"), &env),
        Some(ReasoningMode::Off)
    );
}

#[test]
fn resolve_reasoning_mode_is_none_without_a_supported_value() {
    assert_eq!(resolve_reasoning_mode(None, &lookup(&[])), None);
    let invalid = lookup(&[("GRAYSWAN_REASONING_MODE", "bogus")]);
    assert_eq!(resolve_reasoning_mode(None, &invalid), None);
    let empty = lookup(&[("GRAYSWAN_REASONING_MODE", "")]);
    assert_eq!(resolve_reasoning_mode(None, &empty), None);
}

#[test]
fn is_grayswan_exception_accepts_a_blocked_api_error() {
    assert!(is_grayswan_exception(GuardrailException::Api(
        &blocked_api_error()
    )));
}

#[test]
fn is_grayswan_exception_rejects_an_api_error_without_a_block() {
    let bare = GraySwanGuardrailAPIError {
        message: "boom".to_owned(),
        status_code: Some(500),
        cause: None,
    };
    assert!(!is_grayswan_exception(GuardrailException::Api(&bare)));
    let status = GraySwanGuardrailAPIError::from_failure(GraySwanFailure::HttpStatus {
        status: StatusCode::INTERNAL_SERVER_ERROR,
    });
    assert!(!is_grayswan_exception(GuardrailException::Api(&status)));
}

#[test]
fn is_grayswan_exception_accepts_a_passthrough_modification() {
    let failure = GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
        message: "modified".to_owned(),
        model: "gpt-4o".to_owned(),
        request_data: BTreeMap::new(),
        guardrail_name: None,
        detection_info: BTreeMap::new(),
        original_response: None,
    }));
    assert!(is_grayswan_exception(GuardrailException::Failure(&failure)));
}

#[test]
fn is_grayswan_exception_accepts_an_http_object_naming_the_block() {
    let failure = GuardrailFailure::HttpObject {
        status: 400,
        detail: IndexMap::from([("error".to_owned(), text(GRAYSWAN_BLOCK_ERROR_MSG))]),
    };
    assert!(is_grayswan_exception(GuardrailException::Failure(&failure)));
}

#[test]
fn is_grayswan_exception_rejects_an_http_object_without_the_block_error() {
    let other = GuardrailFailure::HttpObject {
        status: 400,
        detail: IndexMap::from([("error".to_owned(), text("other"))]),
    };
    assert!(!is_grayswan_exception(GuardrailException::Failure(&other)));
    let empty = GuardrailFailure::HttpObject {
        status: 400,
        detail: IndexMap::new(),
    };
    assert!(!is_grayswan_exception(GuardrailException::Failure(&empty)));
}

#[rstest]
#[case::raised(GuardrailFailure::GuardrailRaised {
    guardrail_name: None,
    message: "m".to_owned(),
    wrap_default_message: false,
    status_code: 400,
    blocked_content: false,
})]
#[case::pii(GuardrailFailure::BlockedPii {
    entity_type: "EMAIL_ADDRESS".to_owned(),
    guardrail_name: None,
    status_code: 400,
})]
#[case::route(GuardrailFailure::SensitiveDataRoute(Box::new(SensitiveDataRouteFailure {
    route_to_model: "gpt-4o".to_owned(),
    session_id: "session".to_owned(),
    guardrail_name: None,
    detection_info: BTreeMap::new(),
    message: None,
    sticky_session_routing: false,
})))]
#[case::http(GuardrailFailure::Http {
    status: 500,
    detail: "boom".to_owned(),
})]
#[case::other(GuardrailFailure::Other("boom".to_owned()))]
fn is_grayswan_exception_rejects_other_failures(#[case] failure: GuardrailFailure) {
    assert!(!is_grayswan_exception(GuardrailException::Failure(
        &failure
    )));
}
