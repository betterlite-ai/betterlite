#[test]
fn fetch_listing_reads_models_with_the_bearer_key() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"object":"list","data":[{"id":"gpt-4o","source_model":"openai/gpt-4o"},{"id":"claude"},{"id":"null-source","source_model":null}]}"#,
    )]);
    let models = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap();
    assert_eq!(
        models,
        vec![
            ListedModel {
                id: "gpt-4o".to_owned(),
                source_model: Some("openai/gpt-4o".to_owned()),
            },
            ListedModel {
                id: "claude".to_owned(),
                source_model: None,
            },
            ListedModel {
                id: "null-source".to_owned(),
                source_model: None,
            },
        ]
    );
    let request = get.requests().pop().unwrap();
    assert_eq!(request.url, "https://proxy.test/v1/models");
    assert_eq!(
        request.headers,
        vec![("Authorization".to_owned(), format!("Bearer {API_KEY}"))]
    );
    assert_eq!(request.timeout, Duration::from_secs(10));
}

#[test]
fn fetch_listing_sends_extra_headers_after_authorization() {
    let get = ScriptedGet::new(vec![body(200, r#"{"data":[{"id":"a"}]}"#)]);
    let extra_headers = [
        ("anthropic-version", "2023-06-01"),
        ("x-gateway-client", "claude-code"),
    ];
    fetch_model_listing(BASE_URL, &api_key(), &get, &extra_headers).unwrap();
    let request = get.requests().pop().unwrap();
    assert_eq!(
        request.headers,
        vec![
            ("Authorization".to_owned(), format!("Bearer {API_KEY}")),
            ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
            ("x-gateway-client".to_owned(), "claude-code".to_owned()),
        ]
    );
}

#[test]
fn fetch_listing_lets_an_extra_header_override_authorization() {
    let get = ScriptedGet::new(vec![body(200, r#"{"data":[{"id":"a"}]}"#)]);
    let extra_headers = [("Authorization", "Bearer other")];
    fetch_model_ids(BASE_URL, &api_key(), &get, &extra_headers).unwrap();
    let request = get.requests().pop().unwrap();
    assert_eq!(
        request.headers,
        vec![("Authorization".to_owned(), "Bearer other".to_owned())]
    );
}

#[rstest]
#[case::no_slash("https://proxy.test", "https://proxy.test/v1/models")]
#[case::one_slash("https://proxy.test/", "https://proxy.test/v1/models")]
#[case::many_slashes("https://proxy.test///", "https://proxy.test/v1/models")]
#[case::mounted_path("https://proxy.test/litellm/", "https://proxy.test/litellm/v1/models")]
fn fetch_listing_strips_trailing_slashes(#[case] base_url: &str, #[case] url: &str) {
    let get = ScriptedGet::new(vec![body(200, r#"{"data":[{"id":"a"}]}"#)]);
    fetch_model_listing(base_url, &api_key(), &get, &[]).unwrap();
    assert_eq!(get.urls(), vec![url.to_owned()]);
}

#[test]
fn fetch_listing_dedupes_identical_models() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"id":"a","source_model":"x"},{"id":"a","source_model":"x"}]}"#,
    )]);
    let models = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap();
    assert_eq!(models.len(), 1);
}

#[test]
fn fetch_listing_keeps_a_model_repeated_with_the_same_mapping() {
    // A missing source_model maps to the id, so both entries map "a" to "a" and stay
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"id":"a"},{"id":"a","source_model":"a"}]}"#,
    )]);
    let models = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap();
    assert_eq!(models.len(), 2);
}

#[test]
fn fetch_listing_model_ids_dedupes_ids() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"id":"a"},{"id":"a","source_model":"a"},{"id":"b"}]}"#,
    )]);
    assert_eq!(
        fetch_model_ids(BASE_URL, &api_key(), &get, &[]).unwrap(),
        vec!["a".to_owned(), "b".to_owned()]
    );
}

#[test]
fn fetch_listing_model_ids_reports_a_listing_failure() {
    let get = ScriptedGet::new(vec![body(403, "{}")]);
    let error = fetch_model_ids(BASE_URL, &api_key(), &get, &[]).unwrap_err();
    assert_eq!(error.kind(), Some(ListingFailure::Rejected));
    assert_eq!(error.status(), Some(403));
}

#[test]
fn fetch_listing_rejects_an_empty_model_id() {
    let get = ScriptedGet::new(vec![body(200, r#"{"data":[{"id":""}]}"#)]);
    let error = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap_err();
    assert_eq!(error.kind(), Some(ListingFailure::BadBody));
    assert!(error.to_string().contains("model id is empty"));
}

#[test]
fn fetch_listing_rejects_an_empty_source_model() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"id":"a","source_model":""}]}"#,
    )]);
    let error = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap_err();
    assert_eq!(error.kind(), Some(ListingFailure::BadBody));
    assert!(error.to_string().contains("source_model is empty"));
}

#[rstest]
#[case::not_json("not json")]
#[case::not_an_object("[]")]
#[case::missing_data("{}")]
#[case::data_not_an_array(r#"{"data":{}}"#)]
#[case::missing_id(r#"{"data":[{"source_model":"a"}]}"#)]
#[case::numeric_id(r#"{"data":[{"id":1}]}"#)]
#[case::numeric_source_model(r#"{"data":[{"id":"a","source_model":2}]}"#)]
fn fetch_listing_rejects_an_invalid_body(#[case] text: &str) {
    let get = ScriptedGet::new(vec![body(200, text)]);
    let error = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap_err();
    assert_eq!(error.kind(), Some(ListingFailure::BadBody));
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::unauthorized(401)]
#[case::forbidden(403)]
fn failure_classification_rejected(#[case] status: u16) {
    let get = ScriptedGet::new(vec![body(status, r#"{"data":[{"id":"a"}]}"#)]);
    let error = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap_err();
    assert_eq!(error.kind(), Some(ListingFailure::Rejected));
    assert_eq!(error.status(), Some(status));
    assert!(error.to_string().contains(&status.to_string()));
}

#[rstest]
#[case::redirect(302)]
#[case::not_found(404)]
#[case::server_error(500)]
fn failure_classification_other_status(#[case] status: u16) {
    let get = ScriptedGet::new(vec![body(status, "{}")]);
    let error = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap_err();
    assert_eq!(error.kind(), Some(ListingFailure::Other));
    assert_eq!(error.status(), Some(status));
}

#[test]
fn failure_classification_unreachable_has_no_status() {
    let get = ScriptedGet::new(vec![GetAnswer::Transport("connection refused")]);
    let error = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap_err();
    assert_eq!(error.kind(), Some(ListingFailure::Unreachable));
    assert_eq!(error.status(), None);
    assert!(error.to_string().contains("connection refused"));
}

#[test]
fn failure_classification_bad_body_wins_over_the_status() {
    let get = ScriptedGet::new(vec![body(200, "not json")]);
    let error = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap_err();
    assert_eq!(error.kind(), Some(ListingFailure::BadBody));
    assert!(matches!(error, PiSyncError::ListingJson { .. }));
}

#[test]
fn failure_classification_bad_body_for_a_non_utf8_body() {
    let get = ScriptedGet::new(vec![GetAnswer::Response(200, vec![0xff, 0xfe])]);
    let error = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap_err();
    assert_eq!(error.kind(), Some(ListingFailure::BadBody));
    assert!(matches!(error, PiSyncError::ListingJson { .. }));
}

#[test]
fn failure_classification_empty_for_an_empty_data_array() {
    let get = ScriptedGet::new(vec![body(200, r#"{"data":[]}"#)]);
    let error = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap_err();
    assert_eq!(error.kind(), Some(ListingFailure::Empty));
    assert_eq!(error.status(), None);
    assert!(error.to_string().contains("no models"));
}
