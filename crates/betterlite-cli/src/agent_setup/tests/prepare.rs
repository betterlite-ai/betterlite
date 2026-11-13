fn claude_run<'run>(
    ctx: &'run CliContext,
    environment: &'run FakeEnvironment,
    model: Option<&'run str>,
) -> Run<'run> {
    Run {
        context: ctx,
        targets: vec![Target::Claude],
        model,
        default_model: false,
        mode: APPLY,
        environment,
        stdin_is_terminal: false,
        codex_version: CodexVersion::Reported(Some(SAFE_CODEX)),
    }
}

fn codex_run<'run>(ctx: &'run CliContext, environment: &'run FakeEnvironment) -> Run<'run> {
    let mut run = claude_run(ctx, environment, None);
    run.targets = vec![Target::Codex];
    run
}

async fn execute(
    run: &Run<'_>,
    prompter: &mut ScriptedPrompter,
) -> (Result<(), AgentSetupError>, Vec<u8>, Vec<u8>) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run.execute(prompter, &mut stdout, &mut stderr).await;
    (result, stdout, stderr)
}

fn url_call(message: &str, default: &str) -> Call {
    Call::Url {
        message: message.to_owned(),
        default: default.to_owned(),
    }
}

fn secret_call(message: &str) -> Call {
    Call::Secret {
        message: message.to_owned(),
    }
}

async fn saved_profile(
    environment: &FakeEnvironment,
    target: Target,
    base_url: &str,
    api_key: &str,
    model: Option<&str>,
) {
    let settings_path = settings_path_for(target, environment).unwrap();
    let saved = saved_setup(target, &settings_path, base_url, api_key, model);
    save(&saved, environment).await;
}

#[tokio::test]
async fn prepare_uses_the_model_option_and_starts_on_its_id() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, Some("sonnet"));
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.model.as_deref(), Some("sonnet"));
    assert_eq!(profile.base_url, proxy.base_url);
    assert_eq!(profile.api_key.expose_secret(), "sk-test");
    let settings = claude_settings(&environment);
    assert_eq!(
        env_string(&settings, "ANTHROPIC_MODEL").as_deref(),
        Some("claude-1")
    );
}

#[tokio::test]
async fn prepare_validates_a_claude_id_and_saves_its_source_name() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, Some("claude-1"));
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.model.as_deref(), Some("sonnet"));
}

#[tokio::test]
async fn prepare_reuses_the_saved_model_and_key() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    saved_profile(
        &environment,
        Target::Claude,
        &proxy.base_url,
        "sk-saved",
        Some("sonnet"),
    )
    .await;
    let ctx = context(&proxy.base_url, None);
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, None);
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.model.as_deref(), Some("sonnet"));
    assert_eq!(profile.api_key.expose_secret(), "sk-saved");
    let requests = proxy.requests();
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-saved"));
}

#[tokio::test]
async fn prepare_default_model_unpins_the_saved_model() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    saved_profile(
        &environment,
        Target::Claude,
        &proxy.base_url,
        "sk-saved",
        Some("sonnet"),
    )
    .await;
    let ctx = context(&proxy.base_url, None);
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let mut run = claude_run(&ctx, &environment, None);
    run.default_model = true;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert!(profile.model.is_none());
    let settings = claude_settings(&environment);
    assert_eq!(env_string(&settings, "ANTHROPIC_MODEL"), None);
}

#[tokio::test]
async fn prepare_codex_requires_a_model_before_any_request() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("codex-1", None)]))]).await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let mut run = codex_run(&ctx, &environment);
    run.mode = EDIT;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    let error = result.unwrap_err();
    let AgentSetupError::Usage(text) = &error else {
        panic!("expected a usage error, got {error:?}");
    };
    assert!(text.contains("--model"));
    assert!(text.contains("First-time Codex setup needs a starting model"));
    assert!(proxy.requests().is_empty());
    assert!(prompter.calls.is_empty());
    assert!(
        !settings_path_for(Target::Codex, &environment)
            .unwrap()
            .exists()
    );
}

#[tokio::test]
async fn prepare_edit_connection_offers_the_context_url_and_keeps_the_supplied_key() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("claude-1", None)]))]).await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(vec![
        Reply::Url(proxy.base_url.clone()),
        Reply::Secret(String::new()),
    ]);
    let mut run = claude_run(&ctx, &environment, Some("claude-1"));
    run.mode = EDIT;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    assert_eq!(
        prompter.calls,
        vec![
            url_call("Claude gateway URL", &proxy.base_url),
            secret_call(KEEP_CURRENT_KEY),
        ]
    );
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.base_url, proxy.base_url);
    assert_eq!(profile.api_key.expose_secret(), "sk-test");
}

#[tokio::test]
async fn prepare_edit_connection_offers_the_saved_url_and_keeps_the_saved_key() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    saved_profile(
        &environment,
        Target::Claude,
        &proxy.base_url,
        "sk-saved",
        Some("sonnet"),
    )
    .await;
    let ctx = implicit_context("http://context.test", None);
    let mut prompter = ScriptedPrompter::new(vec![
        Reply::Url(proxy.base_url.clone()),
        Reply::Secret(String::new()),
    ]);
    let mut run = claude_run(&ctx, &environment, None);
    run.mode = EDIT;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    assert_eq!(
        prompter.calls,
        vec![
            url_call("Claude gateway URL", &proxy.base_url),
            secret_call(KEEP_CURRENT_KEY),
        ]
    );
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.api_key.expose_secret(), "sk-saved");
    assert_eq!(
        proxy.requests()[0].header("authorization"),
        Some("Bearer sk-saved")
    );
}

#[tokio::test]
async fn prepare_edit_connection_prefers_an_explicit_context_url() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    saved_profile(
        &environment,
        Target::Claude,
        "http://saved.test",
        "sk-saved",
        Some("sonnet"),
    )
    .await;
    let ctx = context(&proxy.base_url, None);
    let mut prompter = ScriptedPrompter::new(vec![
        Reply::Url(proxy.base_url.clone()),
        Reply::Secret("sk-new".to_owned()),
    ]);
    let mut run = claude_run(&ctx, &environment, None);
    run.mode = EDIT;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    assert_eq!(
        prompter.calls[0],
        url_call("Claude gateway URL", &proxy.base_url)
    );
    assert_eq!(prompter.calls[1], secret_call(NEW_KEY));
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.base_url, proxy.base_url);
    assert_eq!(profile.api_key.expose_secret(), "sk-new");
}

#[tokio::test]
async fn prepare_edit_connection_strips_one_trailing_v1() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("claude-1", None)]))]).await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(vec![
        Reply::Url(format!("{}/v1", proxy.base_url)),
        Reply::Secret(String::new()),
    ]);
    let mut run = claude_run(&ctx, &environment, Some("claude-1"));
    run.mode = EDIT;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.base_url, proxy.base_url);
    assert_eq!(proxy.requests()[0].path, "/v1/models");
}

#[tokio::test]
async fn prepare_edit_connection_rejects_an_invalid_url_as_usage() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("claude-1", None)]))]).await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(vec![Reply::Url("proxy.example.com".to_owned())]);
    let mut run = claude_run(&ctx, &environment, Some("claude-1"));
    run.mode = EDIT;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    let error = result.unwrap_err();
    let AgentSetupError::Usage(text) = &error else {
        panic!("expected a usage error, got {error:?}");
    };
    assert!(text.contains("full http:// or https:// URL including a host"));
    assert!(proxy.requests().is_empty());
}

#[tokio::test]
async fn prepare_edit_connection_asks_again_for_an_empty_new_key() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("claude-1", None)]))]).await;
    let ctx = context(&proxy.base_url, None);
    let mut prompter = ScriptedPrompter::new(vec![
        Reply::Url(proxy.base_url.clone()),
        Reply::Secret(String::new()),
        Reply::Secret("sk-new".to_owned()),
    ]);
    let mut run = claude_run(&ctx, &environment, Some("claude-1"));
    run.mode = EDIT;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    assert_eq!(prompter.calls[1], secret_call(NEW_KEY));
    assert_eq!(prompter.calls[2], secret_call(NEW_KEY));
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.api_key.expose_secret(), "sk-new");
    assert_eq!(
        proxy.requests()[0].header("authorization"),
        Some("Bearer sk-new")
    );
}

#[tokio::test]
async fn prepare_connection_reports_a_changed_gateway() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    saved_profile(
        &environment,
        Target::Claude,
        "http://old.test",
        "sk-saved",
        Some("sonnet"),
    )
    .await;
    let ctx = context(&proxy.base_url, None);
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, None);
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    let error = result.unwrap_err();
    assert!(matches!(error, AgentSetupError::GatewayChanged { .. }));
    let text = error.to_string();
    assert!(text.contains("The gateway changed"));
    assert!(text.contains("--api-key"));
    assert!(text.contains("saved key was not used"));
    assert!(proxy.requests().is_empty());
}

#[tokio::test]
async fn prepare_connection_uses_a_supplied_key_for_a_new_gateway() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    saved_profile(
        &environment,
        Target::Claude,
        "http://old.test",
        "sk-saved",
        Some("sonnet"),
    )
    .await;
    let ctx = context(&proxy.base_url, Some("sk-new"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, None);
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.base_url, proxy.base_url);
    assert_eq!(profile.api_key.expose_secret(), "sk-new");
    assert_eq!(
        proxy.requests()[0].header("authorization"),
        Some("Bearer sk-new")
    );
}

#[tokio::test]
async fn prepare_connection_prefers_the_saved_url_when_the_context_is_implicit() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    saved_profile(
        &environment,
        Target::Claude,
        &proxy.base_url,
        "sk-saved",
        Some("sonnet"),
    )
    .await;
    let ctx = implicit_context("http://context.test", None);
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, None);
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.base_url, proxy.base_url);
    assert_eq!(proxy.requests().len(), 1);
}

#[tokio::test]
async fn prepare_connection_reports_a_bad_supplied_key_for_a_new_gateway() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    saved_profile(
        &environment,
        Target::Claude,
        "http://old.test",
        "sk-saved",
        Some("sonnet"),
    )
    .await;
    let ctx = context(&proxy.base_url, Some("  "));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, None);
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    assert!(matches!(
        result.unwrap_err(),
        AgentSetupError::InvalidCredential
    ));
    assert!(proxy.requests().is_empty());
}

#[tokio::test]
async fn prepare_interactive_picks_a_claude_source_name() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet")), ("claude-2", None)]),
    )])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(vec![Reply::Model("sonnet".to_owned())]);
    let mut run = claude_run(&ctx, &environment, None);
    run.mode = WIZARD;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    assert_eq!(
        prompter.calls,
        vec![Call::Model {
            message: CLAUDE_MODEL_MESSAGE.to_owned(),
            choices: vec![
                KEEP_CLAUDE_DEFAULT.to_owned(),
                "sonnet".to_owned(),
                "claude-2".to_owned(),
            ],
            selected: 0,
        }]
    );
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.model.as_deref(), Some("sonnet"));
    let settings = claude_settings(&environment);
    assert_eq!(
        env_string(&settings, "ANTHROPIC_MODEL").as_deref(),
        Some("claude-1")
    );
}

#[tokio::test]
async fn prepare_interactive_preselects_a_served_default() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet")), ("claude-2", None)]),
    )])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(vec![Reply::Model("claude-2".to_owned())]);
    let mut run = claude_run(&ctx, &environment, Some("claude-2"));
    run.mode = WIZARD;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let Call::Model { selected, .. } = &prompter.calls[0] else {
        panic!("expected a model picker");
    };
    assert_eq!(*selected, 2);
    let settings = claude_settings(&environment);
    assert_eq!(
        env_string(&settings, "ANTHROPIC_MODEL").as_deref(),
        Some("claude-2")
    );
}

#[tokio::test]
async fn prepare_interactive_keeps_claude_codes_default() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(vec![Reply::Model(KEEP_CLAUDE_DEFAULT.to_owned())]);
    let mut run = claude_run(&ctx, &environment, None);
    run.mode = WIZARD;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert!(profile.model.is_none());
}

#[tokio::test]
async fn prepare_interactive_picks_a_codex_id() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("codex-a", None), ("codex-b", None)]),
    )])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(vec![Reply::Model("codex-b".to_owned())]);
    let mut run = codex_run(&ctx, &environment);
    run.mode = WIZARD;
    let (result, _stdout, stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    assert!(stderr.is_empty());
    assert_eq!(
        prompter.calls,
        vec![Call::Model {
            message: CODEX_MODEL_MESSAGE.to_owned(),
            choices: vec!["codex-a".to_owned(), "codex-b".to_owned()],
            selected: 0,
        }]
    );
    let profile = read_saved_setup(Target::Codex, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.model.as_deref(), Some("codex-b"));
    let document = codex_document(&environment);
    assert_eq!(codex_string(&document, "model").as_deref(), Some("codex-b"));
}

#[tokio::test]
async fn prepare_non_interactive_never_asks_for_a_model() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, Some("sonnet"));
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    assert!(prompter.calls.is_empty());
}

#[tokio::test]
async fn prepare_repair_model_asks_when_the_saved_model_is_not_served() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    saved_profile(
        &environment,
        Target::Claude,
        &proxy.base_url,
        "sk-saved",
        Some("gone"),
    )
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(vec![Reply::Model("sonnet".to_owned())]);
    let mut run = claude_run(&ctx, &environment, None);
    run.stdin_is_terminal = true;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    assert_eq!(
        prompter.calls,
        vec![Call::Model {
            message: CLAUDE_MODEL_MESSAGE.to_owned(),
            choices: vec![KEEP_CLAUDE_DEFAULT.to_owned(), "sonnet".to_owned()],
            selected: 0,
        }]
    );
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.model.as_deref(), Some("sonnet"));
}

#[tokio::test]
async fn prepare_repair_codex_asks_when_the_saved_model_is_not_served() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("codex-a", None)]))]).await;
    saved_profile(
        &environment,
        Target::Codex,
        &proxy.base_url,
        "sk-saved",
        Some("gone"),
    )
    .await;
    let ctx = context(&proxy.base_url, None);
    let mut prompter = ScriptedPrompter::new(vec![Reply::Model("codex-a".to_owned())]);
    let mut run = codex_run(&ctx, &environment);
    run.stdin_is_terminal = true;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    assert_eq!(
        prompter.calls,
        vec![Call::Model {
            message: CODEX_MODEL_MESSAGE.to_owned(),
            choices: vec!["codex-a".to_owned()],
            selected: 0,
        }]
    );
    let profile = read_saved_setup(Target::Codex, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.model.as_deref(), Some("codex-a"));
}

#[tokio::test]
async fn prepare_repair_relists_with_a_replacement_key() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![
        Answer::Status(401, "{}".to_owned()),
        Answer::Status(200, listing(&[("claude-1", Some("sonnet"))])),
    ])
    .await;
    saved_profile(
        &environment,
        Target::Claude,
        &proxy.base_url,
        "sk-saved",
        Some("sonnet"),
    )
    .await;
    let ctx = context(&proxy.base_url, Some("sk-old"));
    let mut prompter = ScriptedPrompter::new(vec![Reply::Secret("sk-new".to_owned())]);
    let mut run = claude_run(&ctx, &environment, None);
    run.stdin_is_terminal = true;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    assert_eq!(prompter.calls, vec![secret_call("Replacement virtual key")]);
    let requests = proxy.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-old"));
    assert_eq!(requests[1].header("authorization"), Some("Bearer sk-new"));
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.api_key.expose_secret(), "sk-new");
}

#[tokio::test]
async fn prepare_repair_needs_a_saved_setup() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(401, "{}".to_owned())]).await;
    let ctx = context(&proxy.base_url, Some("sk-old"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let mut run = claude_run(&ctx, &environment, Some("claude-1"));
    run.stdin_is_terminal = true;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    assert!(matches!(
        result.unwrap_err(),
        AgentSetupError::ListingRejected { .. }
    ));
    assert!(prompter.calls.is_empty());
    assert_eq!(proxy.requests().len(), 1);
}

#[tokio::test]
async fn prepare_repair_needs_a_non_interactive_run() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(401, "{}".to_owned())]).await;
    saved_profile(
        &environment,
        Target::Claude,
        &proxy.base_url,
        "sk-saved",
        Some("sonnet"),
    )
    .await;
    let ctx = context(&proxy.base_url, Some("sk-old"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let mut run = claude_run(&ctx, &environment, None);
    run.mode = WIZARD;
    run.stdin_is_terminal = true;
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    assert!(matches!(
        result.unwrap_err(),
        AgentSetupError::ListingRejected { .. }
    ));
    assert!(prompter.calls.is_empty());
    assert_eq!(proxy.requests().len(), 1);
}

#[tokio::test]
async fn prepare_repair_needs_a_terminal() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(401, "{}".to_owned())]).await;
    saved_profile(
        &environment,
        Target::Claude,
        &proxy.base_url,
        "sk-saved",
        Some("sonnet"),
    )
    .await;
    let ctx = context(&proxy.base_url, Some("sk-old"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, None);
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    assert!(matches!(
        result.unwrap_err(),
        AgentSetupError::ListingRejected { .. }
    ));
    assert!(prompter.calls.is_empty());
    assert_eq!(proxy.requests().len(), 1);
}

#[tokio::test]
async fn prepare_rejects_a_profile_the_writer_refuses() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("bad\u{1}model", None)]),
    )])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, Some("bad\u{1}model"));
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    let error = result.unwrap_err();
    assert!(matches!(error, AgentSetupError::InvalidSetup { .. }));
    assert!(
        error
            .to_string()
            .contains("Invalid gateway setup; no settings were changed")
    );
    assert!(
        read_saved_setup(Target::Claude, &environment)
            .unwrap()
            .is_none()
    );
    assert!(
        !settings_path_for(Target::Claude, &environment)
            .unwrap()
            .exists()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_stores_the_resolved_settings_path() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new().unwrap();
    fs::create_dir_all(home.path().join("real")).unwrap();
    symlink(home.path().join("real"), home.path().join("link")).unwrap();
    let environment = FakeEnvironment::with_home(&home.path().join("link"));
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("claude-1", None)]))]).await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, Some("claude-1"));
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let settings_path = settings_path_for(Target::Claude, &environment).unwrap();
    let expected = fs::canonicalize(&settings_path).unwrap();
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.settings_path, expected.to_str().unwrap());
}
