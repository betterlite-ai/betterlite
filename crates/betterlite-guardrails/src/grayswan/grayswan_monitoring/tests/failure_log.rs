#[test]
fn log_guardrail_failure_writes_a_failed_to_respond_record() {
    let mut data = Object::new();
    let started = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    let finished = SystemTime::UNIX_EPOCH + Duration::from_millis(10_500);
    log_guardrail_failure(
        &super::base(),
        GuardrailFailureLog {
            message: "boom".to_owned(),
            status_code: Some(404),
            started_at: started,
            finished_at: finished,
            request_data: &mut data,
            log_target: None,
        },
    );
    let entry = records(&data).first().expect("a guardrail record");
    assert_eq!(field(entry, "guardrail_response"), Some(&text("boom")));
    assert_eq!(
        field(entry, "guardrail_status"),
        Some(&text("guardrail_failed_to_respond"))
    );
    assert_eq!(field(entry, "guardrail_provider"), Some(&text("grayswan")));
    assert_eq!(
        field(entry, "start_time"),
        Some(&GuardrailValue::Decimal(10.0))
    );
    assert_eq!(
        field(entry, "end_time"),
        Some(&GuardrailValue::Decimal(10.5))
    );
    assert_eq!(
        field(entry, "duration"),
        Some(&GuardrailValue::Decimal(0.5))
    );
}

#[test]
fn log_guardrail_failure_keeps_existing_metadata() {
    let mut data = Object::from([(
        "metadata".to_owned(),
        object(vec![("request-id", text("r1"))]),
    )]);
    log_failure(&mut data, None);
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        panic!("metadata");
    };
    assert_eq!(metadata.get("request-id"), Some(&text("r1")));
    assert_eq!(records(&data).len(), 1);
}

#[test]
fn log_guardrail_failure_logs_the_status_code_when_present() {
    let captured = super::Captured::default();
    with_default(
        super::subscriber(captured.clone(), tracing::Level::ERROR),
        || {
            log_failure(&mut Object::new(), Some(404));
        },
    );
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("Gray Swan Guardrail: API request failed (status_code=404): boom"));
}

#[test]
fn log_guardrail_failure_omits_the_status_code_when_absent() {
    let captured = super::Captured::default();
    with_default(
        super::subscriber(captured.clone(), tracing::Level::ERROR),
        || {
            log_failure(&mut Object::new(), None);
        },
    );
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("Gray Swan Guardrail: API request failed: boom"));
    assert!(!output.contains("status_code"));
}
