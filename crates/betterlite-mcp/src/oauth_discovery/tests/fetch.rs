#[rstest]
#[case::not_found(StatusCode::NOT_FOUND, DiscoveryStatus::NotFound)]
#[case::method_not_allowed(StatusCode::METHOD_NOT_ALLOWED, DiscoveryStatus::NotFound)]
#[case::gone(StatusCode::GONE, DiscoveryStatus::NotFound)]
#[case::server_error(
    StatusCode::INTERNAL_SERVER_ERROR,
    DiscoveryStatus::HttpError(StatusCode::INTERNAL_SERVER_ERROR)
)]
#[case::unauthorized(
    StatusCode::UNAUTHORIZED,
    DiscoveryStatus::HttpError(StatusCode::UNAUTHORIZED)
)]
#[case::redirect(StatusCode::FOUND, DiscoveryStatus::HttpError(StatusCode::FOUND))]
fn fetch_classify_status_maps_absence_and_other_errors(
    #[case] status: StatusCode,
    #[case] expected: DiscoveryStatus,
) {
    assert_eq!(classify_status(status), expected);
}

#[rstest]
#[case::not_found(StatusCode::NOT_FOUND)]
#[case::method_not_allowed(StatusCode::METHOD_NOT_ALLOWED)]
#[case::gone(StatusCode::GONE)]
#[tokio::test]
async fn fetch_absent_documents_are_not_found(#[case] status: StatusCode) {
    let fetcher = FakeFetcher::new(vec![("https://mcp.test/prm", status_document(status))]);
    let result =
        fetch_oauth_discovery_url(&fetcher, "https://mcp.test/prm", Duration::from_secs(10)).await;
    assert_eq!(result.unwrap_err(), DiscoveryStatus::NotFound);
}

#[rstest]
#[case::server_error(StatusCode::INTERNAL_SERVER_ERROR)]
#[case::unauthorized(StatusCode::UNAUTHORIZED)]
#[case::redirect(StatusCode::FOUND)]
#[tokio::test]
async fn fetch_other_statuses_are_recorded(#[case] status: StatusCode) {
    let fetcher = FakeFetcher::new(vec![("https://mcp.test/prm", status_document(status))]);
    let result =
        fetch_oauth_discovery_url(&fetcher, "https://mcp.test/prm", Duration::from_secs(10)).await;
    assert_eq!(result.unwrap_err(), DiscoveryStatus::HttpError(status));
}

#[tokio::test]
async fn fetch_transport_errors_carry_sanitized_text() {
    let fetcher = FakeFetcher::new(vec![(
        "https://mcp.test/prm",
        transport_error("connect to https://mcp.test/prm?token=abc failed"),
    )]);
    let result =
        fetch_oauth_discovery_url(&fetcher, "https://mcp.test/prm", Duration::from_secs(10)).await;
    assert_eq!(
        result.unwrap_err(),
        DiscoveryStatus::Transport(String::from("connect to <url> failed"))
    );
}

#[rstest]
#[case::array("[]")]
#[case::array_of_arrays("[[\"https://idp.test\"]]")]
#[case::text("\"metadata\"")]
#[case::null("null")]
#[case::number("7")]
#[case::empty("")]
#[case::truncated("{\"authorization_servers\": [")]
#[tokio::test]
async fn fetch_a_body_that_is_not_an_object_is_invalid(#[case] body: &str) {
    let fetcher = FakeFetcher::new(vec![("https://mcp.test/prm", document(body))]);
    let result =
        fetch_oauth_discovery_url(&fetcher, "https://mcp.test/prm", Duration::from_secs(10)).await;
    assert_eq!(result.unwrap_err(), DiscoveryStatus::Invalid);
}

#[rstest]
#[case::servers_type("{\"authorization_servers\": \"https://idp.test\"}")]
#[case::scope_entry_type("{\"scopes_supported\": [\"read\", 5]}")]
#[case::scopes_type("{\"scopes_supported\": 5}")]
#[case::endpoint_type("{\"token_endpoint\": 5}")]
#[case::resource_type("{\"resource\": 5}")]
#[tokio::test]
async fn fetch_wrong_typed_fields_are_invalid(#[case] body: &str) {
    let fetcher = FakeFetcher::new(vec![("https://mcp.test/prm", document(body))]);
    let result =
        fetch_oauth_discovery_url(&fetcher, "https://mcp.test/prm", Duration::from_secs(10)).await;
    assert_eq!(result.unwrap_err(), DiscoveryStatus::Invalid);
}

#[test]
fn fetch_an_oversized_document_is_rejected_before_decoding() {
    let body = vec![b' '; MAX_METADATA_BYTES.saturating_add(1)];
    assert!(matches!(
        decode_metadata_document(&body),
        Err(DocumentError::TooLarge)
    ));
}

#[test]
fn fetch_a_document_at_the_size_limit_is_read() {
    let padding = "a".repeat(MAX_METADATA_BYTES.saturating_sub(15));
    let body = format!("{{\"resource\":\"{padding}\"}}");
    assert_eq!(body.len(), MAX_METADATA_BYTES);
    decode_metadata_document(body.as_bytes()).unwrap();
}

#[tokio::test]
async fn fetch_passes_the_timeout_and_no_headers_to_the_port() {
    let fetcher = FakeFetcher::new(vec![("https://mcp.test/prm", document("{}"))]);
    let timeout = Duration::from_millis(250);
    fetch_oauth_discovery_url(&fetcher, "https://mcp.test/prm", timeout)
        .await
        .unwrap();
    let calls = fetcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].timeout, timeout);
    assert!(calls[0].headers.is_empty());
}

#[tokio::test]
async fn fetch_decodes_a_protected_resource_document() {
    let body = "{\"resource\":\"https://mcp.test/mcp\",\
                \"authorization_servers\":[\"https://idp.test\",\"\",5,\"https://idp2.test\"],\
                \"scopes_supported\":[\"read\",\"write\"]}";
    let fetcher = FakeFetcher::new(vec![("https://mcp.test/prm", document(body))]);
    let resource = fetch_resource_metadata(&fetcher, "https://mcp.test/prm", settings())
        .await
        .unwrap();
    assert_eq!(
        resource,
        ResourceMetadata {
            authorization_servers: strings(&["https://idp.test", "https://idp2.test"]),
            scopes_supported: Some(strings(&["read", "write"])),
            resource: Some(String::from("https://mcp.test/mcp")),
        }
    );
}

#[rstest]
#[case::list("[\"read\", \"write\"]", Some(vec!["read", "write"]))]
#[case::text("\"read write\"", Some(vec!["read", "write"]))]
#[case::empty_list("[]", None)]
#[case::blank_entry("[\"  \"]", None)]
#[case::null("null", None)]
fn fetch_scopes_values_are_extracted(#[case] value: &str, #[case] expected: Option<Vec<&str>>) {
    let body = format!("{{\"scopes_supported\":{value}}}");
    let document = decode_metadata_document(body.as_bytes()).unwrap();
    assert_eq!(
        document_scopes(document.scopes_supported.as_ref()),
        expected.map(|values| strings(&values))
    );
}

#[test]
fn fetch_an_explicit_empty_scopes_list_wins_over_the_scopes_key() {
    let document =
        decode_metadata_document(b"{\"scopes_supported\":[],\"scopes\":[\"read\"]}").unwrap();
    assert_eq!(
        document_scopes(
            document
                .scopes_supported
                .as_ref()
                .or(document.scopes.as_ref())
        ),
        None
    );
    assert_eq!(document.scopes, Some(ScopesValue::List(strings(&["read"]))));
}

#[test]
fn fetch_a_resource_is_kept_as_text() {
    let document = decode_metadata_document(b"{\"resource\":\"https://mcp.test\"}").unwrap();
    assert_eq!(document.resource.as_deref(), Some("https://mcp.test"));
}

#[rstest]
#[case::one_url(
    "connect to https://idp.test/token?secret=1 failed",
    "connect to <url> failed"
)]
#[case::both_schemes("http://a.test/x then https://b.test/y", "<url> then <url>")]
#[case::uppercase_scheme("HTTP://a.test/x", "HTTP://a.test/x")]
#[case::marker_at_the_end("see https://", "see https://")]
#[case::marker_inside_a_word("xhttp://a.test/y", "x<url>")]
fn fetch_sanitized_error_text_replaces_urls(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(sanitized_error_text(text), expected);
}

#[test]
fn fetch_sanitized_error_text_keeps_two_hundred_characters() {
    let text = "a".repeat(260);
    assert_eq!(sanitized_error_text(&text).chars().count(), 200);
}

#[test]
fn fetch_redacted_origin_list_joins_redacted_origins() {
    let urls = strings(&["https://idp.test/token?x=1", "not a url"]);
    assert_eq!(
        redacted_origin_list(&urls),
        "https://idp.test, <unparsable url>"
    );
}
