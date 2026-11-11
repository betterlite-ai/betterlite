const INSTALL_COMMAND: &str = "/usr/local/bin/betterlite statusline";

fn prepare_request<'request>(
    api_key: &'request SecretString,
    environment: &'request FakeEnvironment,
) -> PrepareRequest<'request> {
    PrepareRequest {
        base_url: BASE_URL,
        api_key,
        environment,
    }
}

fn codex_environment(root: &Path) -> FakeEnvironment {
    FakeEnvironment {
        variables: vars(&[("CODEX_HOME", root.to_str().unwrap())]),
        home: None,
    }
}

fn installed_hook() -> String {
    format!(
        "hooks.Stop=[{{hooks=[{{type=\"command\",command={}}}]}}]",
        serde_json::json!(INSTALL_COMMAND)
    )
}

#[test]
fn prepare_codex_installs_the_stop_hook() {
    let directory = temp_dir();
    let environment = codex_environment(directory.path());
    let installer = ScriptedInstaller::installing(INSTALL_COMMAND);
    let preparer = CodexPreparer {
        install: &installer,
    };
    let api_key = key();
    let request = prepare_request(&api_key, &environment);
    let mut messages = RecordingMessages::default();
    let args = preparer.prepare(&request, &mut messages).unwrap();
    assert_eq!(args, ["-c".to_owned(), installed_hook()]);
    assert_eq!(installer.calls(), 1_usize);
    assert!(messages.warns().is_empty());
}

#[test]
fn prepare_codex_warns_when_the_config_declares_stop_hooks() {
    let directory = temp_dir();
    fs::write(
        directory.path().join("config.toml"),
        "hooks.Stop = [{ hooks = [] }]\n",
    )
    .unwrap();
    let environment = codex_environment(directory.path());
    let installer = ScriptedInstaller::installing(INSTALL_COMMAND);
    let preparer = CodexPreparer {
        install: &installer,
    };
    let api_key = key();
    let request = prepare_request(&api_key, &environment);
    let mut messages = RecordingMessages::default();
    let args = preparer.prepare(&request, &mut messages).unwrap();
    assert!(args.is_empty());
    assert_eq!(installer.calls(), 0_usize);
    assert_eq!(
        messages.warns(),
        [
            "litellm: your Codex config already declares hooks; not adding the routed-model Stop hook"
        ]
    );
}

#[test]
fn prepare_codex_installs_when_the_config_cannot_be_read() {
    let directory = temp_dir();
    fs::write(directory.path().join("config.toml"), [0xff_u8, 0xfe]).unwrap();
    let environment = codex_environment(directory.path());
    let installer = ScriptedInstaller::installing(INSTALL_COMMAND);
    let preparer = CodexPreparer {
        install: &installer,
    };
    let api_key = key();
    let request = prepare_request(&api_key, &environment);
    let mut messages = RecordingMessages::default();
    let args = preparer.prepare(&request, &mut messages).unwrap();
    assert_eq!(args.len(), 2_usize);
    assert_eq!(installer.calls(), 1_usize);
    assert!(messages.warns().is_empty());
}

#[test]
fn prepare_codex_wraps_an_installer_error() {
    let directory = temp_dir();
    let environment = codex_environment(directory.path());
    let installer = ScriptedInstaller::failing("no statusline for you");
    let preparer = CodexPreparer {
        install: &installer,
    };
    let api_key = key();
    let request = prepare_request(&api_key, &environment);
    let mut messages = RecordingMessages::default();
    let error = preparer.prepare(&request, &mut messages).unwrap_err();
    let AgentRunError::Settings(source) = error else {
        panic!("expected a settings error");
    };
    assert_eq!(
        source.to_string(),
        "cannot locate the betterlite executable: no statusline for you"
    );
}

#[test]
fn prepare_codex_uses_the_home_config_when_codex_home_is_unset() {
    let directory = temp_dir();
    let home = directory.path().join("home");
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".codex").join("config.toml"), "hooks.Stop = []\n").unwrap();
    let environment = FakeEnvironment {
        variables: EnvVars::new(),
        home: Some(home),
    };
    let installer = ScriptedInstaller::installing(INSTALL_COMMAND);
    let preparer = CodexPreparer {
        install: &installer,
    };
    let api_key = key();
    let request = prepare_request(&api_key, &environment);
    let mut messages = RecordingMessages::default();
    let args = preparer.prepare(&request, &mut messages).unwrap();
    assert!(args.is_empty());
    assert_eq!(installer.calls(), 0_usize);
    assert_eq!(messages.warns().len(), 1_usize);
}

#[test]
fn prepare_codex_installs_without_a_home_directory() {
    let environment = FakeEnvironment {
        variables: EnvVars::new(),
        home: None,
    };
    let installer = ScriptedInstaller::installing(INSTALL_COMMAND);
    let preparer = CodexPreparer {
        install: &installer,
    };
    let api_key = key();
    let request = prepare_request(&api_key, &environment);
    let mut messages = RecordingMessages::default();
    let args = preparer.prepare(&request, &mut messages).unwrap();
    assert_eq!(args.len(), 2_usize);
    assert_eq!(installer.calls(), 1_usize);
}

fn pi_environment(root: &Path) -> (EnvVars, FakeEnvironment) {
    let pi_directory = root.join("pi");
    let variables = vars(&[("PI_CODING_AGENT_DIR", pi_directory.to_str().unwrap())]);
    let environment = FakeEnvironment {
        variables: variables.clone(),
        home: None,
    };
    (variables, environment)
}

fn pi_http() -> ScriptedHttp {
    ScriptedHttp::new(vec![
        ScriptedHttp::body(200, &models_body(r#"[{"id":"gpt-4o"},{"id":"claude"}]"#)),
        ScriptedHttp::body(
            200,
            &limits_body(
                r#"[{"model_group":"gpt-4o","max_input_tokens":128000,"max_output_tokens":4096}]"#,
            ),
        ),
    ])
}

fn run_pi_prepare(root: &Path) -> (Vec<String>, RecordingMessages, PathBuf) {
    let (variables, environment) = pi_environment(root);
    let http = pi_http();
    let files = ScriptedFiles::working();
    let preparer = PiPreparer {
        get: &http,
        files: &files,
    };
    let api_key = key();
    let request = prepare_request(&api_key, &environment);
    let mut messages = RecordingMessages::default();
    let args = preparer.prepare(&request, &mut messages).unwrap();
    let path = Path::new(&variables["PI_CODING_AGENT_DIR"]).join("models.json");
    (args, messages, path)
}

#[test]
fn prepare_pi_syncs_models_and_pins_the_first_one() {
    let directory = temp_dir();
    let (args, messages, path) = run_pi_prepare(directory.path());
    assert_eq!(args, ["--model".to_owned(), "litellm/gpt-4o".to_owned()]);
    assert_eq!(
        messages.notices(),
        [format!(
            "litellm: synced 2 proxy models into {}",
            path.display()
        )]
    );
}

#[test]
fn prepare_pi_writes_the_provider_block() {
    let directory = temp_dir();
    let (_args, _messages, path) = run_pi_prepare(directory.path());
    let document = read_models_file(&path);
    let provider = &document.providers["litellm"];
    assert_eq!(provider.base_url, "http://gateway.test:4000/v1");
    assert_eq!(provider.api, "openai-completions");
    assert_eq!(provider.api_key, "$LITELLM_PROXY_API_KEY");
    assert_eq!(provider.models.len(), 2_usize);
    assert_eq!(provider.models[0].id, "gpt-4o");
    assert_eq!(provider.models[0].context_window, Some(128_000_i64));
    assert_eq!(provider.models[0].max_tokens, Some(4096_i64));
    assert_eq!(provider.models[1].id, "claude");
    assert_eq!(provider.models[1].context_window, None);
}

#[test]
fn prepare_pi_reads_the_model_limits_after_the_listing() {
    let directory = temp_dir();
    let variables = pi_environment(directory.path()).0;
    let http = pi_http();
    let files = ScriptedFiles::working();
    let environment = FakeEnvironment {
        variables,
        home: None,
    };
    let preparer = PiPreparer {
        get: &http,
        files: &files,
    };
    let api_key = key();
    let request = prepare_request(&api_key, &environment);
    let mut messages = RecordingMessages::default();
    let _unused = preparer.prepare(&request, &mut messages).unwrap();
    let calls = http.calls();
    assert_eq!(calls.len(), 2_usize);
    assert_eq!(calls[0].url, "http://gateway.test:4000/v1/models");
    assert_eq!(calls[0].authorization, "Bearer sk-test");
    assert_eq!(calls[1].url, "http://gateway.test:4000/model_group/info");
}

#[test]
fn prepare_pi_reports_a_listing_status_failure() {
    let directory = temp_dir();
    let (_variables, environment) = pi_environment(directory.path());
    let http = ScriptedHttp::new(vec![ScriptedHttp::body(401, b"")]);
    let files = ScriptedFiles::working();
    let preparer = PiPreparer {
        get: &http,
        files: &files,
    };
    let api_key = key();
    let request = prepare_request(&api_key, &environment);
    let mut messages = RecordingMessages::default();
    let error = preparer.prepare(&request, &mut messages).unwrap_err();
    let AgentRunError::Pi(source) = error else {
        panic!("expected a pi sync error");
    };
    assert_eq!(source.status(), Some(401_u16));
    assert!(source.to_string().contains("HTTP 401"));
    assert!(messages.notices().is_empty());
}

#[test]
fn prepare_pi_reports_an_empty_listing_as_nothing_to_run() {
    let directory = temp_dir();
    let (_variables, environment) = pi_environment(directory.path());
    let http = ScriptedHttp::new(vec![ScriptedHttp::body(200, &models_body("[]"))]);
    let files = ScriptedFiles::working();
    let preparer = PiPreparer {
        get: &http,
        files: &files,
    };
    let api_key = key();
    let request = prepare_request(&api_key, &environment);
    let mut messages = RecordingMessages::default();
    let error = preparer.prepare(&request, &mut messages).unwrap_err();
    let AgentRunError::Pi(source) = error else {
        panic!("expected a pi sync error");
    };
    assert!(matches!(source, PiSyncError::NothingToRun));
    assert!(source.to_string().contains("pi would have nothing to run"));
}

#[test]
fn prepare_pi_without_a_config_directory_is_an_error() {
    let environment = FakeEnvironment {
        variables: EnvVars::new(),
        home: None,
    };
    let http = pi_http();
    let files = ScriptedFiles::working();
    let preparer = PiPreparer {
        get: &http,
        files: &files,
    };
    let api_key = key();
    let request = prepare_request(&api_key, &environment);
    let mut messages = RecordingMessages::default();
    let error = preparer.prepare(&request, &mut messages).unwrap_err();
    let AgentRunError::Pi(source) = error else {
        panic!("expected a pi sync error");
    };
    assert!(matches!(source, PiSyncError::NoHome));
}

#[test]
fn preparers_map_pi_and_codex_by_command_name() {
    let directory = temp_dir();
    let installer = ScriptedInstaller::installing(INSTALL_COMMAND);
    let codex = CodexPreparer {
        install: &installer,
    };
    let http = pi_http();
    let files = ScriptedFiles::working();
    let pi = PiPreparer {
        get: &http,
        files: &files,
    };
    let preparers = Preparers {
        pi: &pi,
        codex: &codex,
    };
    assert!(preparers.get("pi").is_some());
    assert!(preparers.get("codex").is_some());
    assert!(preparers.get("claude").is_none());
    assert!(preparers.get("codex.cmd").is_none());
    assert!(preparers.get("/opt/codex").is_none());
    let (_variables, pi_environment) = pi_environment(directory.path());
    let api_key = key();
    let request = prepare_request(&api_key, &pi_environment);
    let mut messages = RecordingMessages::default();
    let args = preparers
        .get("pi")
        .unwrap()
        .prepare(&request, &mut messages)
        .unwrap();
    assert_eq!(args, ["--model".to_owned(), "litellm/gpt-4o".to_owned()]);
}

#[test]
fn base_environment_reads_the_agent_variables_and_the_given_home() {
    let variables = vars(&[("CODEX_HOME", "/x/codex")]);
    let home = Path::new("/home/tester");
    let environment = BaseEnvironment {
        variables: &variables,
        home: Some(home),
    };
    assert_eq!(
        environment.lookup("CODEX_HOME"),
        Some("/x/codex".to_owned())
    );
    assert_eq!(environment.lookup("PI_HOME"), None);
    assert_eq!(environment.system_home(), Some(home.to_path_buf()));
    let homeless = BaseEnvironment {
        variables: &variables,
        home: None,
    };
    assert_eq!(homeless.system_home(), None);
}
