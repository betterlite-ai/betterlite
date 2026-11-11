async fn fetch(base_url: &str, target: Target) -> Result<Vec<ListedModel>, AgentSetupError> {
    let environment = FakeEnvironment::without_home();
    fetch_listing(base_url, &token("sk-test"), target, &environment).await
}

#[tokio::test]
async fn listing_routes_through_the_environment_proxy() {
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("claude-3", None)]))]).await;
    let environment =
        FakeEnvironment::without_home().with_variable("HTTP_PROXY", "http://127.0.0.1:1");
    let error = fetch_listing(
        &proxy.base_url,
        &token("sk-test"),
        Target::Claude,
        &environment,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AgentSetupError::ListingUnreachable { .. }));
    assert!(proxy.requests().is_empty());
}

#[tokio::test]
async fn listing_sends_the_claude_code_headers() {
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("claude-3", None)]))]).await;
    let models = fetch(&proxy.base_url, Target::Claude).await.unwrap();
    assert_eq!(models.len(), 1);
    let requests = proxy.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/models");
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-test"));
    assert_eq!(requests[0].header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(requests[0].header("x-gateway-client"), Some("claude-code"));
}

#[tokio::test]
async fn listing_sends_only_the_bearer_for_codex() {
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("codex-1", None)]))]).await;
    let models = fetch(&proxy.base_url, Target::Codex).await.unwrap();
    assert_eq!(models.len(), 1);
    let requests = proxy.requests();
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-test"));
    assert_eq!(requests[0].header("anthropic-version"), None);
    assert_eq!(requests[0].header("x-gateway-client"), None);
}

#[tokio::test]
async fn listing_does_not_follow_a_redirect() {
    let proxy = MockProxy::start(vec![
        Answer::Redirect("/redirected".to_owned()),
        Answer::Status(200, listing(&[("claude-3", None)])),
    ])
    .await;
    let error = fetch(&proxy.base_url, Target::Claude).await.unwrap_err();
    assert!(matches!(error, AgentSetupError::ListingFailed { .. }));
    let requests = proxy.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/models");
}

#[tokio::test]
async fn listing_reports_a_rejected_key_401() {
    let proxy = MockProxy::start(vec![Answer::Status(401, "{}".to_owned())]).await;
    let error = fetch(&proxy.base_url, Target::Claude).await.unwrap_err();
    let AgentSetupError::ListingRejected { status, .. } = &error else {
        panic!("expected a rejection, got {error:?}");
    };
    assert_eq!(*status, 401);
    let text = error.to_string();
    assert!(text.contains("HTTP 401"));
    assert!(text.contains("--api-key"));
}

#[tokio::test]
async fn listing_reports_a_rejected_key_403() {
    let proxy = MockProxy::start(vec![Answer::Status(403, "{}".to_owned())]).await;
    let error = fetch(&proxy.base_url, Target::Claude).await.unwrap_err();
    let AgentSetupError::ListingRejected { status, .. } = &error else {
        panic!("expected a rejection, got {error:?}");
    };
    assert_eq!(*status, 403);
}

#[tokio::test]
async fn listing_reports_another_status_as_a_proxy_health_problem() {
    let proxy = MockProxy::start(vec![Answer::Status(500, "{}".to_owned())]).await;
    let error = fetch(&proxy.base_url, Target::Claude).await.unwrap_err();
    assert!(matches!(error, AgentSetupError::ListingFailed { .. }));
    let text = error.to_string();
    assert!(text.contains(&proxy.base_url));
    assert!(text.contains("healthy"));
}

#[tokio::test]
async fn listing_reports_a_bad_body_as_a_proxy_health_problem() {
    let proxy = MockProxy::start(vec![Answer::Status(200, "not json".to_owned())]).await;
    let error = fetch(&proxy.base_url, Target::Claude).await.unwrap_err();
    assert!(matches!(error, AgentSetupError::ListingFailed { .. }));
}

#[tokio::test]
async fn listing_empty_for_claude_names_claude_code() {
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[]))]).await;
    let error = fetch(&proxy.base_url, Target::Claude).await.unwrap_err();
    assert!(matches!(error, AgentSetupError::ListingEmpty { .. }));
    let text = error.to_string();
    assert!(text.contains("no models for your key"));
    assert!(text.contains("Claude Code would have nothing to run"));
    assert!(text.contains("give the key access to at least one model"));
}

#[tokio::test]
async fn listing_empty_for_codex_names_codex() {
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[]))]).await;
    let error = fetch(&proxy.base_url, Target::Codex).await.unwrap_err();
    assert!(matches!(error, AgentSetupError::ListingEmpty { .. }));
    assert!(
        error
            .to_string()
            .contains("Codex would have nothing to run")
    );
}

#[tokio::test]
async fn listing_unreachable_names_the_proxy_and_the_url_options() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let base_url = format!("http://{address}");
    let error = fetch(&base_url, Target::Claude).await.unwrap_err();
    let AgentSetupError::ListingUnreachable {
        base_url: reported, ..
    } = &error
    else {
        panic!("expected an unreachable proxy, got {error:?}");
    };
    assert_eq!(reported, &base_url);
    let text = error.to_string();
    assert!(text.contains(&base_url));
    assert!(text.contains("running"));
    assert!(text.contains("--base-url"));
    assert!(text.contains("LITELLM_PROXY_URL"));
}

#[tokio::test]
async fn listing_accepts_a_body_exactly_at_the_cap() {
    let json = "{\"data\":[]}";
    let mut body = " ".repeat(MAX_LISTING_BYTES.saturating_sub(json.len()));
    body.push_str(json);
    assert_eq!(body.len(), MAX_LISTING_BYTES);
    let proxy = MockProxy::start(vec![Answer::Status(200, body)]).await;
    let error = fetch(&proxy.base_url, Target::Claude).await.unwrap_err();
    assert!(
        matches!(error, AgentSetupError::ListingEmpty { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn listing_rejects_a_body_over_the_cap() {
    let body = "x".repeat(MAX_LISTING_BYTES.saturating_add(1));
    let proxy = MockProxy::start(vec![Answer::Status(200, body)]).await;
    let error = fetch(&proxy.base_url, Target::Claude).await.unwrap_err();
    assert!(matches!(error, AgentSetupError::ListingTooLarge { .. }));
    assert!(error.to_string().contains(&proxy.base_url));
}

#[tokio::test]
async fn listing_repair_skips_a_non_rejected_failure() {
    let proxy = MockProxy::start(vec![Answer::Status(500, "{}".to_owned())]).await;
    let ctx = context(&proxy.base_url, Some("sk-old"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let environment = FakeEnvironment::without_home();
    let error = {
        let mut request = ListingRequest {
            context: &ctx,
            base_url: &proxy.base_url,
            credential: token("sk-old"),
            target: Target::Claude,
            repair: true,
            environment: &environment,
            prompter: &mut prompter,
        };
        connection_listing(&mut request).await.unwrap_err()
    };
    assert!(matches!(error, AgentSetupError::ListingFailed { .. }));
    assert!(prompter.calls.is_empty());
    assert_eq!(proxy.requests().len(), 1);
}

#[test]
fn listing_validates_a_source_model_before_an_id() {
    let models = listed(&[("claude-a", Some("sonnet")), ("sonnet", None)]);
    assert_eq!(starting_model("sonnet", &models), Some("claude-a"));
    assert_eq!(
        validated_model(Some("sonnet"), &models, "http://proxy.test").unwrap(),
        Some("claude-a")
    );
}

#[test]
fn listing_validates_an_id_when_no_source_model_matches() {
    let models = listed(&[("claude-a", Some("sonnet"))]);
    assert_eq!(
        validated_model(Some("claude-a"), &models, "http://proxy.test").unwrap(),
        Some("claude-a")
    );
}

#[test]
fn listing_starts_on_the_first_source_match() {
    let models = listed(&[("claude-a", Some("sonnet")), ("claude-b", Some("sonnet"))]);
    assert_eq!(starting_model("sonnet", &models), Some("claude-a"));
}

#[test]
fn listing_rejects_an_unserved_model_with_the_first_twenty_ids() {
    let ids: Vec<String> = (0_i32..25_i32)
        .map(|index| format!("model-{index}"))
        .collect();
    let models: Vec<ListedModel> = ids
        .iter()
        .map(|id| ListedModel {
            id: id.clone(),
            source_model: None,
        })
        .collect();
    let error = validated_model(Some("missing"), &models, "http://proxy.test").unwrap_err();
    assert!(matches!(error, AgentSetupError::ModelNotServed { .. }));
    let text = error.to_string();
    assert!(text.contains("missing"));
    assert!(text.contains("http://proxy.test"));
    let shown = ids[..20].join(", ");
    assert!(text.contains(&shown));
    assert!(!text.contains("model-20"));
}

#[tokio::test]
async fn listing_repair_relists_with_the_replacement_key() {
    let proxy = MockProxy::start(vec![
        Answer::Status(401, "{}".to_owned()),
        Answer::Status(200, listing(&[("claude-3", None)])),
    ])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-old"));
    let mut prompter = ScriptedPrompter::new(vec![Reply::Secret("sk-new".to_owned())]);
    let environment = FakeEnvironment::without_home();
    let (active, models) = {
        let mut request = ListingRequest {
            context: &ctx,
            base_url: &proxy.base_url,
            credential: token("sk-old"),
            target: Target::Claude,
            repair: true,
            environment: &environment,
            prompter: &mut prompter,
        };
        connection_listing(&mut request).await.unwrap()
    };
    assert_eq!(active.token.expose_secret(), "sk-new");
    assert_eq!(models.len(), 1);
    assert_eq!(
        prompter.calls,
        vec![Call::Secret {
            message: "Replacement virtual key".to_owned()
        }]
    );
    let requests = proxy.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-old"));
    assert_eq!(requests[1].header("authorization"), Some("Bearer sk-new"));
}

#[tokio::test]
async fn listing_repair_asks_again_for_an_empty_replacement() {
    let proxy = MockProxy::start(vec![
        Answer::Status(401, "{}".to_owned()),
        Answer::Status(200, listing(&[("claude-3", None)])),
    ])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-old"));
    let mut prompter = ScriptedPrompter::new(vec![
        Reply::Secret(String::new()),
        Reply::Secret("sk-new".to_owned()),
    ]);
    let environment = FakeEnvironment::without_home();
    let (active, _models) = {
        let mut request = ListingRequest {
            context: &ctx,
            base_url: &proxy.base_url,
            credential: token("sk-old"),
            target: Target::Claude,
            repair: true,
            environment: &environment,
            prompter: &mut prompter,
        };
        connection_listing(&mut request).await.unwrap()
    };
    assert_eq!(active.token.expose_secret(), "sk-new");
    assert_eq!(prompter.calls.len(), 2);
}

#[tokio::test]
async fn listing_repair_off_reports_the_rejection() {
    let proxy = MockProxy::start(vec![Answer::Status(401, "{}".to_owned())]).await;
    let ctx = context(&proxy.base_url, Some("sk-old"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let environment = FakeEnvironment::without_home();
    let error = {
        let mut request = ListingRequest {
            context: &ctx,
            base_url: &proxy.base_url,
            credential: token("sk-old"),
            target: Target::Claude,
            repair: false,
            environment: &environment,
            prompter: &mut prompter,
        };
        connection_listing(&mut request).await.unwrap_err()
    };
    assert!(matches!(error, AgentSetupError::ListingRejected { .. }));
    assert!(prompter.calls.is_empty());
    assert_eq!(proxy.requests().len(), 1);
}

#[tokio::test]
async fn listing_repair_second_failure_uses_its_own_kind() {
    let proxy = MockProxy::start(vec![
        Answer::Status(401, "{}".to_owned()),
        Answer::Status(500, "{}".to_owned()),
    ])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-old"));
    let mut prompter = ScriptedPrompter::new(vec![Reply::Secret("sk-new".to_owned())]);
    let environment = FakeEnvironment::without_home();
    let error = {
        let mut request = ListingRequest {
            context: &ctx,
            base_url: &proxy.base_url,
            credential: token("sk-old"),
            target: Target::Claude,
            repair: true,
            environment: &environment,
            prompter: &mut prompter,
        };
        connection_listing(&mut request).await.unwrap_err()
    };
    assert!(matches!(error, AgentSetupError::ListingFailed { .. }));
    assert_eq!(proxy.requests().len(), 2);
    assert_eq!(prompter.calls.len(), 1);
}

#[tokio::test]
async fn listing_repair_rejects_a_bad_replacement_key() {
    let proxy = MockProxy::start(vec![Answer::Status(401, "{}".to_owned())]).await;
    let ctx = context(&proxy.base_url, Some("sk-old"));
    let mut prompter = ScriptedPrompter::new(vec![
        Reply::Secret(String::new()),
        Reply::Secret("  ".to_owned()),
    ]);
    let environment = FakeEnvironment::without_home();
    let error = {
        let mut request = ListingRequest {
            context: &ctx,
            base_url: &proxy.base_url,
            credential: token("sk-old"),
            target: Target::Claude,
            repair: true,
            environment: &environment,
            prompter: &mut prompter,
        };
        connection_listing(&mut request).await.unwrap_err()
    };
    assert!(matches!(error, AgentSetupError::InvalidCredential));
    assert_eq!(proxy.requests().len(), 1);
    assert_eq!(prompter.calls.len(), 2);
}
