fn both_targets<'run>(
    ctx: &'run CliContext,
    environment: &'run FakeEnvironment,
    model: &'run str,
    version: CodexVersion<'run>,
) -> Run<'run> {
    Run {
        context: ctx,
        targets: vec![Target::Claude, Target::Codex],
        model: Some(model),
        default_model: false,
        mode: APPLY,
        environment,
        stdin_is_terminal: false,
        codex_version: version,
    }
}

fn assert_no_setup(environment: &FakeEnvironment) {
    for target in [Target::Claude, Target::Codex] {
        assert!(read_saved_setup(target, environment).unwrap().is_none());
        assert!(!settings_path_for(target, environment).unwrap().exists());
    }
}

fn assert_apply_failure(error: &AgentSetupError) {
    let AgentSetupError::Apply {
        name,
        target,
        source,
    } = error
    else {
        panic!("expected the apply failure, got {error:?}");
    };
    assert_eq!(*name, "Claude");
    assert_eq!(*target, Target::Claude);
    assert!(matches!(**source, AgentSetupError::Claude(_)));
    let text = error.to_string();
    assert!(text.contains("Claude setup was saved"));
    assert!(text.contains("Run `lite configure claude` to retry applying it"));
}

fn codex_stdout(settings_path: &Path, base_url: &str, model: &str) -> String {
    lines(&[
        format!(
            "Configured Codex: {} now routes through {base_url}.",
            settings_path.display()
        ),
        format!("Starting model: {model}. Credential: your virtual key, stored in the private provider settings."),
        "Start `codex` from any terminal. Undo with `lite unconfigure codex`.".to_owned(),
        "Setup saved. Edit with `lite reconfigure codex`; remove saved settings and key with `lite unconfigure codex --forget`.".to_owned(),
    ])
}

#[tokio::test]
async fn apply_rejects_model_with_default_model() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("claude-1", None)]))]).await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let mut run = claude_run(&ctx, &environment, Some("claude-1"));
    run.default_model = true;
    let (result, stdout, stderr) = execute(&run, &mut prompter).await;
    let error = result.unwrap_err();
    assert!(matches!(error, AgentSetupError::Usage(_)));
    assert!(
        error
            .to_string()
            .contains("--model and --default-model cannot be used together")
    );
    assert!(proxy.requests().is_empty());
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}

#[tokio::test]
async fn apply_preflights_every_target_before_listing() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("shared", None)]))]).await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = both_targets(
        &ctx,
        &environment,
        "shared",
        CodexVersion::Reported(Some("codex-cli 0.128.9")),
    );
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    let error = result.unwrap_err();
    let AgentSetupError::CodexSettings(source) = &error else {
        panic!("expected the Codex preflight, got {error:?}");
    };
    assert!(matches!(**source, CodexSettingsError::UnsafeCodex));
    assert!(proxy.requests().is_empty());
    assert!(
        read_saved_setup(Target::Claude, &environment)
            .unwrap()
            .is_none()
    );
    assert!(
        read_saved_setup(Target::Codex, &environment)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn apply_reports_a_missing_home_before_listing() {
    let environment = FakeEnvironment::without_home();
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("claude-1", None)]))]).await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, Some("claude-1"));
    let (result, _stdout, _stderr) = execute(&run, &mut prompter).await;
    let error = result.unwrap_err();
    let AgentSetupError::Profile(source) = &error else {
        panic!("expected a profile error, got {error:?}");
    };
    assert!(matches!(
        **source,
        AgentSetupProfilesError::Path(Target::Claude)
    ));
    assert!(proxy.requests().is_empty());
}

#[tokio::test]
async fn apply_prepares_every_target_before_saving_any() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![
        Answer::Status(200, listing(&[("shared", None)])),
        Answer::Status(500, "{}".to_owned()),
    ])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = both_targets(
        &ctx,
        &environment,
        "shared",
        CodexVersion::Reported(Some(SAFE_CODEX)),
    );
    let (result, stdout, _stderr) = execute(&run, &mut prompter).await;
    assert!(matches!(
        result.unwrap_err(),
        AgentSetupError::ListingFailed { .. }
    ));
    assert_eq!(proxy.requests().len(), 2);
    assert!(stdout.is_empty());
    assert_no_setup(&environment);
}

#[tokio::test]
async fn apply_saves_the_profile_before_settings_and_reports_it() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-1", Some("sonnet"))]),
    )])
    .await;
    let settings_path = settings_path_for(Target::Claude, &environment).unwrap();
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    let corrupted = settings_path.clone();
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(vec![
        Reply::Url(proxy.base_url.clone()),
        Reply::Secret(String::new()),
        Reply::Model("sonnet".to_owned()),
    ]);
    prompter.after_url = Some(Box::new(move || {
        fs::write(&corrupted, b"{\"env\":\"nope\"}").unwrap();
    }));
    let mut run = claude_run(&ctx, &environment, None);
    run.mode = EDIT_WIZARD;
    let (result, stdout, _stderr) = execute(&run, &mut prompter).await;
    let error = result.unwrap_err();
    assert_apply_failure(&error);
    assert!(stdout.is_empty());
    assert_eq!(fs::read(&settings_path).unwrap(), b"{\"env\":\"nope\"}");
    assert!(
        read_saved_setup(Target::Claude, &environment)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn apply_prints_the_claude_lines() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("claude-3", None)]))]).await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, Some("claude-3"));
    let (result, stdout, stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let settings_path = settings_path_for(Target::Claude, &environment).unwrap();
    let expected = lines(&[
        format!(
            "Configured Claude Code: {} now routes through {}.",
            settings_path.display(),
            proxy.base_url
        ),
        "Credential: your virtual key, stored in the file as ANTHROPIC_AUTH_TOKEN.".to_owned(),
        format!("Starting model: claude-3 ({STARTING_MODEL_ROLE}); switch any time with /model."),
        "/model will list all 1 of the proxy's models.".to_owned(),
        "Start `claude` from any terminal. Undo with `lite unconfigure claude`.".to_owned(),
        "Setup saved. Edit with `lite reconfigure claude`; remove saved settings and key with `lite unconfigure claude --forget`.".to_owned(),
    ]);
    assert_eq!(String::from_utf8(stdout).unwrap(), expected);
    assert!(stderr.is_empty());
    let settings = claude_settings(&environment);
    assert_eq!(
        env_string(&settings, "ANTHROPIC_BASE_URL").as_deref(),
        Some(proxy.base_url.as_str())
    );
    assert_eq!(
        env_string(&settings, "ANTHROPIC_AUTH_TOKEN").as_deref(),
        Some("sk-test")
    );
}

#[tokio::test]
async fn apply_prints_the_unpinned_claude_line() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("claude-3", None)]))]).await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, None);
    let (result, stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let text = String::from_utf8(stdout).unwrap();
    assert!(text.contains(
        "Starting model: not pinned (Claude Code's default, or a model you set yourself); switch with /model, or pass --model to start on a proxy model. Without a pin, a resumed session re-sends the model its transcript recorded, which behind a raw-model auto-router is the tier model."
    ));
    assert!(text.contains("/model will list all 1 of the proxy's models."));
    let settings = claude_settings(&environment);
    assert_eq!(env_string(&settings, "ANTHROPIC_MODEL"), None);
}

#[tokio::test]
async fn apply_counts_picker_models_case_insensitively() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[
            ("Claude-3", None),
            ("my-anthropic-proxy", None),
            ("gpt-4", None),
        ]),
    )])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, Some("Claude-3"));
    let (result, stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let text = String::from_utf8(stdout).unwrap();
    assert!(text.contains(
        "/model will list 2 of the proxy's 3 models: Claude Code shows only ids containing 'claude' or 'anthropic', and this proxy does not list the rest under such names."
    ));
    assert!(!text.contains("/model will list all 3"));
}

#[cfg(unix)]
#[tokio::test]
async fn apply_prints_the_claude_symlink_note() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let settings_path = settings_path_for(Target::Claude, &environment).unwrap();
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    let real = home.path().join("real-settings.json");
    fs::write(&real, "{}").unwrap();
    symlink(&real, &settings_path).unwrap();
    let proxy = MockProxy::start(vec![Answer::Status(200, listing(&[("claude-1", None)]))]).await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, Some("claude-1"));
    let (result, stdout, stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    assert!(
        String::from_utf8(stdout)
            .unwrap()
            .contains(&settings_path.display().to_string())
    );
    let resolved = fs::canonicalize(&real).unwrap();
    let expected = format!(
        "Note: {} is a symlink to {}, so your key now lives in that file; keep it out of version control.\n",
        settings_path.display(),
        resolved.display()
    );
    assert_eq!(String::from_utf8(stderr).unwrap(), expected);
}

#[cfg(unix)]
#[tokio::test]
async fn apply_prints_the_codex_lines_and_writes_the_model_as_given() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let settings_path = settings_path_for(Target::Codex, &environment).unwrap();
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    let real = home.path().join("real-config.toml");
    fs::write(&real, "").unwrap();
    symlink(&real, &settings_path).unwrap();
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("codex-1", Some("gpt-5"))]),
    )])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let mut run = codex_run(&ctx, &environment);
    run.model = Some("codex-1");
    let (result, stdout, stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let expected = codex_stdout(&settings_path, &proxy.base_url, "codex-1");
    assert_eq!(String::from_utf8(stdout).unwrap(), expected);
    let resolved = fs::canonicalize(&real).unwrap();
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        format!(
            "Note: your key now lives in {}; keep it out of version control.\n",
            resolved.display()
        )
    );
    let document = codex_document(&environment);
    assert_eq!(codex_string(&document, "model").as_deref(), Some("codex-1"));
    let profile = read_saved_setup(Target::Codex, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.model.as_deref(), Some("codex-1"));
}

#[tokio::test]
async fn apply_starts_claude_on_the_first_id_for_a_shared_source() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![Answer::Status(
        200,
        listing(&[("claude-a", Some("sonnet")), ("claude-b", Some("sonnet"))]),
    )])
    .await;
    let ctx = context(&proxy.base_url, Some("sk-test"));
    let mut prompter = ScriptedPrompter::new(Vec::new());
    let run = claude_run(&ctx, &environment, Some("claude-b"));
    let (result, stdout, _stderr) = execute(&run, &mut prompter).await;
    result.unwrap();
    let text = String::from_utf8(stdout).unwrap();
    assert!(text.contains(&format!(
        "Starting model: claude-a ({STARTING_MODEL_ROLE}); switch any time with /model."
    )));
    let settings = claude_settings(&environment);
    assert_eq!(
        env_string(&settings, "ANTHROPIC_MODEL").as_deref(),
        Some("claude-a")
    );
    let profile = read_saved_setup(Target::Claude, &environment)
        .unwrap()
        .unwrap();
    assert_eq!(profile.model.as_deref(), Some("sonnet"));
}
