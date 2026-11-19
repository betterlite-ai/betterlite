#[test]
fn extract_inbound_headers_takes_the_proxy_server_request_headers() {
    let data = Object::from([(
        "proxy_server_request".to_owned(),
        object(vec![(
            "headers",
            object(vec![
                ("shade_scan_id", text("abc")),
                ("x-request-id", text("r")),
            ]),
        )]),
    )]);
    assert_eq!(
        extract_inbound_headers(&data, None),
        Some(BTreeMap::from([(
            "shade_scan_id".to_owned(),
            "abc".to_owned()
        )]))
    );
}

#[test]
fn extract_inbound_headers_prefers_the_proxy_server_request_over_the_request_headers() {
    let data = Object::from([
        (
            "proxy_server_request".to_owned(),
            object(vec![(
                "headers",
                object(vec![("shade_scan_id", text("proxy"))]),
            )]),
        ),
        (
            "headers".to_owned(),
            object(vec![("shade_scan_id", text("request"))]),
        ),
    ]);
    assert_eq!(
        extract_inbound_headers(&data, None),
        Some(BTreeMap::from([(
            "shade_scan_id".to_owned(),
            "proxy".to_owned()
        )]))
    );
}

#[test]
fn extract_inbound_headers_prefers_the_request_headers_over_the_metadata_headers() {
    let data = Object::from([
        (
            "headers".to_owned(),
            object(vec![("shade_scan_id", text("request"))]),
        ),
        (
            "metadata".to_owned(),
            object(vec![(
                "headers",
                object(vec![("shade_scan_id", text("metadata"))]),
            )]),
        ),
    ]);
    assert_eq!(
        extract_inbound_headers(&data, None),
        Some(BTreeMap::from([(
            "shade_scan_id".to_owned(),
            "request".to_owned()
        )]))
    );
}

#[test]
fn extract_inbound_headers_falls_back_to_the_request_headers() {
    let data = Object::from([(
        "headers".to_owned(),
        object(vec![("shade_scan_id", text("request"))]),
    )]);
    assert_eq!(
        extract_inbound_headers(&data, None),
        Some(BTreeMap::from([(
            "shade_scan_id".to_owned(),
            "request".to_owned()
        )]))
    );
}

#[test]
fn extract_inbound_headers_falls_back_to_the_metadata_headers() {
    let data = Object::from([(
        "metadata".to_owned(),
        object(vec![(
            "headers",
            object(vec![("shade_scan_id", text("metadata"))]),
        )]),
    )]);
    assert_eq!(
        extract_inbound_headers(&data, None),
        Some(BTreeMap::from([(
            "shade_scan_id".to_owned(),
            "metadata".to_owned()
        )]))
    );
}

#[test]
fn extract_inbound_headers_prefers_the_metadata_headers_over_the_log_target() {
    let target = log_target(vec![("shade_scan_id", text("logged"))]);
    let data = Object::from([(
        "metadata".to_owned(),
        object(vec![(
            "headers",
            object(vec![("shade_scan_id", text("metadata"))]),
        )]),
    )]);
    assert_eq!(
        extract_inbound_headers(&data, Some(&target)),
        Some(BTreeMap::from([(
            "shade_scan_id".to_owned(),
            "metadata".to_owned()
        )]))
    );
}

#[test]
fn extract_inbound_headers_falls_back_to_the_log_target_case_insensitively() {
    let target = log_target(vec![("Shade_Scan_ID", text("logged"))]);
    assert_eq!(
        extract_inbound_headers(&Object::new(), Some(&target)),
        Some(BTreeMap::from([(
            "Shade_Scan_ID".to_owned(),
            "logged".to_owned()
        )]))
    );
}

#[test]
fn extract_inbound_headers_prefers_the_request_headers_over_the_log_target() {
    let target = log_target(vec![("shade_scan_id", text("logged"))]);
    let data = Object::from([(
        "headers".to_owned(),
        object(vec![("shade_scan_id", text("request"))]),
    )]);
    assert_eq!(
        extract_inbound_headers(&data, Some(&target)),
        Some(BTreeMap::from([(
            "shade_scan_id".to_owned(),
            "request".to_owned()
        )]))
    );
}

#[test]
fn extract_inbound_headers_skips_an_empty_headers_object() {
    let target = log_target(vec![("shade_scan_id", text("logged"))]);
    let data = Object::from([("headers".to_owned(), object(Vec::new()))]);
    assert_eq!(
        extract_inbound_headers(&data, Some(&target)),
        Some(BTreeMap::from([(
            "shade_scan_id".to_owned(),
            "logged".to_owned()
        )]))
    );
}

#[test]
fn extract_inbound_headers_ignores_other_names() {
    assert_eq!(FORWARDED_HEADER_NAMES, ["shade_scan_id"]);
    let data = Object::from([(
        "headers".to_owned(),
        object(vec![("x-request-id", text("r"))]),
    )]);
    assert_eq!(extract_inbound_headers(&data, None), None);
}

#[test]
fn extract_inbound_headers_drops_a_non_text_value() {
    let data = Object::from([(
        "headers".to_owned(),
        object(vec![("shade_scan_id", GuardrailValue::UnsignedInteger(7))]),
    )]);
    assert_eq!(extract_inbound_headers(&data, None), None);
}

#[test]
fn extract_inbound_headers_returns_none_without_headers() {
    assert_eq!(extract_inbound_headers(&Object::new(), None), None);
}

#[test]
fn prepare_headers_sets_the_bearer_token_and_the_api_key() {
    let headers = prepare_headers(&settings()).expect("headers");
    assert_eq!(
        headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
        "Bearer secret"
    );
    assert_eq!(
        headers.get("grayswan-api-key").unwrap().to_str().unwrap(),
        "secret"
    );
    assert_eq!(
        headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
}

#[test]
fn prepare_headers_marks_the_secret_headers_sensitive() {
    let headers = prepare_headers(&settings()).expect("headers");
    assert!(headers.get(AUTHORIZATION).unwrap().is_sensitive());
    assert!(headers.get("grayswan-api-key").unwrap().is_sensitive());
    assert!(!headers.get(CONTENT_TYPE).unwrap().is_sensitive());
}

#[test]
fn prepare_headers_rejects_an_invalid_key() {
    let mut settings = settings();
    settings.api_key = SecretString::from("bad\u{0}key");
    let error = prepare_headers(&settings).expect_err("a NUL byte cannot be a header");
    assert!(matches!(
        error.cause,
        Some(GraySwanFailure::InvalidHeader { .. })
    ));
    assert_eq!(error.status_code, None);
}
