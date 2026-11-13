struct CodexCase {
    _directory: tempfile::TempDir,
    base_env: EnvVars,
    codex_home: PathBuf,
}

impl CodexCase {
    fn new() -> Self {
        let directory = temp_dir();
        let codex_home = directory.path().join("codex");
        let base_env = vars(&[("CODEX_HOME", codex_home.to_str().unwrap())]);
        Self {
            _directory: directory,
            base_env,
            codex_home,
        }
    }

    fn catalog_path(&self) -> PathBuf {
        self.codex_home.join("litellm-models.json")
    }
}

struct SyncFixture {
    case: CodexCase,
    get: ScriptedHttp,
    debug: ScriptedDebug,
    files: ScriptedFiles,
    home: Option<PathBuf>,
}

impl SyncFixture {
    fn new(get: ScriptedHttp, debug: ScriptedDebug, files: ScriptedFiles) -> Self {
        Self {
            case: CodexCase::new(),
            get,
            debug,
            files,
            home: None,
        }
    }

    fn happy() -> Self {
        let listing = models_body(
            r#"[{"id":"gpt-known","mode":"chat"},{"id":"gpt-unknown","mode":"responses","max_input_tokens":200000},{"id":"embed","mode":"embedding"}]"#,
        );
        let get = ScriptedHttp::new(vec![ScriptedHttp::body(200, &listing)]);
        let stock = stock_catalog(&[r#"{"slug":"gpt-known","priority":7,"visibility":"hide","supported_in_api":false,"upgrade":{"model":"gpt-unknown","reason":"newer"},"context_window":128000,"base_instructions":"stock-known"}"#.to_owned()]);
        let debug = ScriptedDebug::new(vec![
            ScriptedDebug::printed(&stock),
            ScriptedDebug::printed("{}"),
        ]);
        Self::new(get, debug, ScriptedFiles::working())
    }

    fn run(&self) -> Result<ModelSyncResult, AgentRunError> {
        let api_key = key();
        let request = CodexSyncRequest {
            base_env: &self.case.base_env,
            base_url: BASE_URL,
            api_key: &api_key,
            binary: CODEX_BINARY,
        };
        let context = SyncContext {
            get: &self.get,
            debug: &self.debug,
            files: &self.files,
            home: self.home.as_deref(),
            instructions: "You are Codex.",
        };
        codex_model_sync_args(&request, &context)
    }

    fn dispatch(&self, binary: &str, skip_verify: bool) -> Result<ModelSyncResult, AgentRunError> {
        let api_key = key();
        let request = SyncRequest {
            binary,
            base_env: &self.case.base_env,
            base_url: BASE_URL,
            api_key: &api_key,
            skip_verify,
        };
        let context = SyncContext {
            get: &self.get,
            debug: &self.debug,
            files: &self.files,
            home: self.home.as_deref(),
            instructions: "You are Codex.",
        };
        agent_model_sync_env(&request, &context)
    }
}

fn expect_args(result: ModelSyncResult) -> Vec<String> {
    if let ModelSyncResult::Args(args) = result {
        args.args
    } else {
        panic!("expected sync arguments");
    }
}

fn expect_skipped(result: ModelSyncResult) -> String {
    if let ModelSyncResult::Skipped(skipped) = result {
        skipped.reason
    } else {
        panic!("expected a skipped sync");
    }
}

fn expect_env(result: ModelSyncResult) -> EnvVars {
    if let ModelSyncResult::Env(env) = result {
        env
    } else {
        panic!("expected a sync environment");
    }
}

#[test]
fn codex_model_sync_args_returns_the_catalog_override() {
    let fixture = SyncFixture::happy();
    let args = expect_args(fixture.run().unwrap());
    let path = fixture.case.catalog_path();
    let override_arg = format!(
        "model_catalog_json={}",
        serde_json::json!(path.to_str().unwrap())
    );
    assert_eq!(args, ["-c".to_owned(), override_arg.clone()]);
    let debug_calls = fixture.debug.calls();
    assert_eq!(debug_calls.len(), 2);
    assert_eq!(
        debug_calls[0].command,
        WindowsCommand::Direct(CODEX_BINARY.to_owned(), command(&["debug", "models"]))
    );
    assert_eq!(
        debug_calls[1].command,
        WindowsCommand::Direct(
            CODEX_BINARY.to_owned(),
            command(&["-c", &override_arg, "debug", "models"])
        )
    );
    assert_eq!(debug_calls[1].timeout, Duration::from_secs(10));
    assert_eq!(debug_calls[0].env, fixture.case.base_env);
    assert_eq!(debug_calls[1].env, fixture.case.base_env);
    let http_calls = fixture.get.calls();
    assert_eq!(http_calls.len(), 1);
    assert_eq!(http_calls[0].url, "http://gateway.test:4000/v1/models");
    assert_eq!(http_calls[0].authorization, "Bearer sk-test");
    assert_eq!(http_calls[0].timeout, Duration::from_secs(10));
    assert_eq!(files_in(&fixture.case.codex_home), ["litellm-models.json"]);
    assert!(
        fixture
            .files
            .staged_paths()
            .iter()
            .all(|staged| !staged.exists())
    );
}

#[test]
fn codex_model_sync_args_merges_stock_and_fallback_models() {
    let fixture = SyncFixture::happy();
    let _unused = expect_args(fixture.run().unwrap());
    let catalog = read_catalog(&fixture.case.catalog_path());
    assert_eq!(
        catalog
            .models
            .iter()
            .map(|model| model.slug.as_str())
            .collect::<Vec<&str>>(),
        ["gpt-known", "gpt-unknown"]
    );
    let known = &catalog.models[0];
    assert_eq!(known.priority, 0_i64);
    assert_eq!(known.visibility, "list");
    assert!(known.supported_in_api);
    assert_eq!(
        known.upgrade,
        Some(CatalogUpgrade {
            model: "gpt-unknown".to_owned(),
            reason: Some("newer".to_owned()),
        })
    );
    assert_eq!(known.context_window, Some(128_000_i64));
    assert_eq!(known.base_instructions.as_deref(), Some("stock-known"));
    let unknown = &catalog.models[1];
    assert_eq!(unknown.priority, 1_i64);
    assert_eq!(unknown.visibility, "list");
    assert!(unknown.supported_in_api);
    assert_eq!(unknown.display_name.as_deref(), Some("gpt-unknown"));
    assert_eq!(unknown.context_window, Some(200_000_i64));
    assert_eq!(unknown.base_instructions.as_deref(), Some("You are Codex."));
    assert_eq!(unknown.upgrade, None);
}

#[test]
fn codex_model_sync_args_skips_when_the_listing_fails() {
    let get = ScriptedHttp::new(vec![ScriptedHttp::body(404, b"")]);
    let fixture = SyncFixture::new(
        get,
        ScriptedDebug::new(Vec::new()),
        ScriptedFiles::working(),
    );
    let reason = expect_skipped(fixture.run().unwrap());
    assert_eq!(
        reason,
        "http://gateway.test:4000/v1/models returned HTTP 404"
    );
    assert!(fixture.debug.calls().is_empty());
    assert!(!fixture.case.catalog_path().exists());
}

#[test]
fn codex_model_sync_args_skips_when_the_listing_is_unreachable() {
    let get = ScriptedHttp::new(vec![HttpAnswer::Failure(io::ErrorKind::ConnectionRefused)]);
    let fixture = SyncFixture::new(
        get,
        ScriptedDebug::new(Vec::new()),
        ScriptedFiles::working(),
    );
    let reason = expect_skipped(fixture.run().unwrap());
    assert!(reason.starts_with("could not reach http://gateway.test:4000/v1/models: "));
    assert!(fixture.debug.calls().is_empty());
}

#[test]
fn codex_model_sync_args_skips_when_no_chat_models_are_served() {
    let get = SyncFixture::happy();
    let _unused = get;
    let fixture = SyncFixture::new(
        ScriptedHttp::new(vec![ScriptedHttp::body(
            200,
            &models_body(r#"[{"id":"embed","mode":"embedding"}]"#),
        )]),
        ScriptedDebug::new(vec![ScriptedDebug::printed(&stock_catalog(&[]))]),
        ScriptedFiles::working(),
    );
    let reason = expect_skipped(fixture.run().unwrap());
    assert_eq!(
        reason,
        "http://gateway.test:4000/v1/models lists no chat models"
    );
    assert_eq!(fixture.debug.calls().len(), 1);
    assert!(!fixture.case.catalog_path().exists());
}

#[test]
fn codex_model_sync_args_skips_without_a_home_directory() {
    let get = ScriptedHttp::new(vec![ScriptedHttp::body(
        200,
        &models_body(r#"[{"id":"gpt-4o","mode":"chat"}]"#),
    )]);
    let debug = ScriptedDebug::new(Vec::new());
    let files = ScriptedFiles::working();
    let api_key = key();
    let base_env = EnvVars::new();
    let request = CodexSyncRequest {
        base_env: &base_env,
        base_url: BASE_URL,
        api_key: &api_key,
        binary: CODEX_BINARY,
    };
    let context = SyncContext {
        get: &get,
        debug: &debug,
        files: &files,
        home: None,
        instructions: "You are Codex.",
    };
    let reason = expect_skipped(codex_model_sync_args(&request, &context).unwrap());
    assert_eq!(
        reason,
        "could not locate a home directory for the Codex model catalog; set CODEX_HOME"
    );
    assert_eq!(get.calls().len(), 1);
    assert!(debug.calls().is_empty());
}

#[test]
fn codex_model_sync_args_skips_when_the_catalog_directory_cannot_be_created() {
    let fixture = SyncFixture::new(
        ScriptedHttp::new(vec![ScriptedHttp::body(
            200,
            &models_body(r#"[{"id":"gpt-4o","mode":"chat"}]"#),
        )]),
        ScriptedDebug::new(Vec::new()),
        ScriptedFiles::failing(FileFailure::Parents),
    );
    let reason = expect_skipped(fixture.run().unwrap());
    assert!(reason.starts_with("could not write "));
    assert!(reason.ends_with("scripted filesystem failure"));
    assert!(fixture.debug.calls().is_empty());
}

#[test]
fn codex_model_sync_args_skips_when_the_catalog_cannot_be_replaced() {
    let fixture = SyncFixture::new(
        ScriptedHttp::new(vec![ScriptedHttp::body(
            200,
            &models_body(r#"[{"id":"gpt-4o","mode":"chat"}]"#),
        )]),
        ScriptedDebug::new(vec![ScriptedDebug::printed(&stock_catalog(&[]))]),
        ScriptedFiles::failing(FileFailure::Replace),
    );
    let reason = expect_skipped(fixture.run().unwrap());
    assert!(reason.starts_with("could not write "));
    assert!(!fixture.case.catalog_path().exists());
    assert!(
        fixture
            .files
            .staged_paths()
            .iter()
            .all(|staged| !staged.exists())
    );
}

#[test]
fn codex_model_sync_args_keeps_the_catalog_when_the_read_back_fails() {
    let fixture = SyncFixture::new(
        ScriptedHttp::new(vec![ScriptedHttp::body(
            200,
            &models_body(r#"[{"id":"gpt-4o","mode":"chat"}]"#),
        )]),
        ScriptedDebug::new(vec![
            ScriptedDebug::printed(&stock_catalog(&[])),
            ScriptedDebug::exited(2_i32, "error: invalid catalog"),
        ]),
        ScriptedFiles::working(),
    );
    let reason = expect_skipped(fixture.run().unwrap());
    assert_eq!(
        reason,
        "`codex debug models` exited 2: error: invalid catalog"
    );
    let catalog = read_catalog(&fixture.case.catalog_path());
    assert_eq!(catalog.models.len(), 1);
    assert_eq!(catalog.models[0].slug, "gpt-4o");
    assert_eq!(fixture.debug.calls().len(), 2);
}

#[test]
fn codex_model_sync_args_uses_the_home_catalog_when_codex_home_is_unset() {
    let directory = temp_dir();
    let home = directory.path().join("home");
    let mut fixture = SyncFixture::new(
        ScriptedHttp::new(vec![ScriptedHttp::body(
            200,
            &models_body(r#"[{"id":"gpt-4o","mode":"chat"}]"#),
        )]),
        ScriptedDebug::new(vec![
            ScriptedDebug::printed(&stock_catalog(&[])),
            ScriptedDebug::printed("{}"),
        ]),
        ScriptedFiles::working(),
    );
    fixture.case.base_env = EnvVars::new();
    fixture.home = Some(home.clone());
    let _unused = expect_args(fixture.run().unwrap());
    assert!(home.join(".codex").join("litellm-models.json").exists());
}

#[cfg(unix)]
#[test]
fn codex_model_sync_args_skips_a_home_that_is_not_utf8() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;
    let directory = temp_dir();
    let home = directory.path().join(OsStr::from_bytes(b"home-\xff"));
    let mut fixture = SyncFixture::new(
        ScriptedHttp::new(vec![ScriptedHttp::body(
            200,
            &models_body(r#"[{"id":"gpt-4o","mode":"chat"}]"#),
        )]),
        ScriptedDebug::new(vec![ScriptedDebug::printed(&stock_catalog(&[]))]),
        ScriptedFiles::working(),
    );
    fixture.case.base_env = EnvVars::new();
    fixture.home = Some(home);
    let reason = expect_skipped(fixture.run().unwrap());
    assert!(reason.starts_with("the Codex model catalog path "));
    assert!(reason.ends_with(" is not valid UTF-8"));
    assert_eq!(fixture.debug.calls().len(), 1);
}

#[test]
fn agent_model_sync_env_returns_opencode_config_content() {
    let fixture = SyncFixture::new(
        ScriptedHttp::new(vec![ScriptedHttp::body(
            200,
            &models_body(
                r#"[{"id":"gpt-4o","mode":"chat","max_input_tokens":128000,"max_output_tokens":4096},{"id":"embed","mode":"embedding"}]"#,
            ),
        )]),
        ScriptedDebug::new(Vec::new()),
        ScriptedFiles::working(),
    );
    let env = expect_env(fixture.dispatch("/usr/local/bin/opencode", false).unwrap());
    let config: OpenCodeConfig = serde_json::from_str(&env["OPENCODE_CONFIG_CONTENT"]).unwrap();
    let provider = &config.provider["litellm"];
    assert_eq!(provider.npm, "@ai-sdk/openai-compatible");
    assert_eq!(provider.name, "LiteLLM");
    assert_eq!(provider.options.base_url, "http://gateway.test:4000/v1");
    assert_eq!(provider.options.api_key, "{env:OPENAI_API_KEY}");
    assert_eq!(provider.models.len(), 1);
    assert_eq!(provider.models["gpt-4o"].name, "gpt-4o");
    let limit = provider.models["gpt-4o"].limit.as_ref().unwrap();
    assert_eq!(limit.context, 128_000_i64);
    assert_eq!(limit.output, 4096_i64);
    assert!(fixture.debug.calls().is_empty());
}

#[test]
fn agent_model_sync_env_skips_opencode_when_config_content_is_already_set() {
    let mut fixture = SyncFixture::new(
        ScriptedHttp::new(Vec::new()),
        ScriptedDebug::new(Vec::new()),
        ScriptedFiles::working(),
    );
    fixture.case.base_env = vars(&[("OPENCODE_CONFIG_CONTENT", "{\"provider\":{}}")]);
    let reason = expect_skipped(fixture.dispatch("opencode", false).unwrap());
    assert_eq!(reason, "OPENCODE_CONFIG_CONTENT is already set");
    assert!(fixture.get.calls().is_empty());
}

#[test]
fn agent_model_sync_env_returns_codex_args() {
    let fixture = SyncFixture::happy();
    let args = expect_args(fixture.dispatch("codex", false).unwrap());
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], "-c");
    assert!(args[1].starts_with("model_catalog_json="));
}

#[test]
fn agent_model_sync_env_dispatches_through_the_binary_extension() {
    let fixture = SyncFixture::happy();
    let _unused = expect_args(fixture.dispatch("/opt/codex.cmd", false).unwrap());
    assert!(matches!(
        fixture.debug.calls()[0].command,
        WindowsCommand::Batch(_)
    ));
}

#[rstest]
#[case::claude("/usr/bin/claude")]
#[case::pi("pi")]
#[case::lookalike("codex-extra")]
#[case::archive("codex.tar.gz")]
fn agent_model_sync_env_returns_an_empty_environment_for_other_agents(#[case] binary: &str) {
    let fixture = SyncFixture::new(
        ScriptedHttp::new(Vec::new()),
        ScriptedDebug::new(Vec::new()),
        ScriptedFiles::working(),
    );
    let env = expect_env(fixture.dispatch(binary, false).unwrap());
    assert!(env.is_empty(), "{binary} added environment");
    assert!(fixture.get.calls().is_empty());
    assert!(fixture.debug.calls().is_empty());
}

#[rstest]
#[case::codex("codex")]
#[case::opencode("opencode")]
fn agent_model_sync_env_skips_syncing_when_verification_is_skipped(#[case] binary: &str) {
    let fixture = SyncFixture::new(
        ScriptedHttp::new(Vec::new()),
        ScriptedDebug::new(Vec::new()),
        ScriptedFiles::working(),
    );
    let reason = expect_skipped(fixture.dispatch(binary, true).unwrap());
    assert_eq!(reason, format!("{SKIP_VERIFY_FLAG} was passed"));
    assert!(fixture.get.calls().is_empty());
    assert!(fixture.debug.calls().is_empty());
}

#[test]
fn the_model_sync_trait_delegates_to_the_dispatch() {
    let fixture = SyncFixture::happy();
    let api_key = key();
    let request = SyncRequest {
        binary: "codex",
        base_env: &fixture.case.base_env,
        base_url: BASE_URL,
        api_key: &api_key,
        skip_verify: false,
    };
    let sync = AgentModelSync {
        context: SyncContext {
            get: &fixture.get,
            debug: &fixture.debug,
            files: &fixture.files,
            home: None,
            instructions: "You are Codex.",
        },
    };
    let args = expect_args(sync.sync(&request).unwrap());
    assert_eq!(args.len(), 2);
}
