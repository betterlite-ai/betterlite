#[test]
fn codex_model_catalog_path_uses_home() {
    let path = codex_model_catalog_path(&EnvVars::new(), Some(Path::new("/home/tester")));
    assert_eq!(
        path,
        Some(PathBuf::from("/home/tester/.codex/litellm-models.json"))
    );
    assert_eq!(CODEX_MODEL_CATALOG_FILENAME, "litellm-models.json");
}

#[test]
fn codex_model_catalog_path_uses_codex_home() {
    let env = vars(&[("CODEX_HOME", "/opt/codex")]);
    let path = codex_model_catalog_path(&env, Some(Path::new("/home/tester")));
    assert_eq!(path, Some(PathBuf::from("/opt/codex/litellm-models.json")));
}

#[test]
fn codex_model_catalog_path_empty_codex_home_uses_home() {
    let env = vars(&[("CODEX_HOME", "")]);
    let path = codex_model_catalog_path(&env, Some(Path::new("/home/tester")));
    assert_eq!(
        path,
        Some(PathBuf::from("/home/tester/.codex/litellm-models.json"))
    );
}

#[test]
fn codex_model_catalog_path_without_home_is_none() {
    assert_eq!(codex_model_catalog_path(&EnvVars::new(), None), None);
}

#[test]
fn replace_file_writes_the_text_and_cleans_the_staged_file() {
    let directory = temp_dir();
    let target = directory.path().join("catalog.json");
    let files = ScriptedFiles::working();
    replace_file(&target, "{\"models\":[]}", &files).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "{\"models\":[]}");
    assert_eq!(files.staged_paths().len(), 1_usize);
    assert_eq!(files.staged_paths()[0].parent(), Some(directory.path()));
    assert!(!files.staged_paths()[0].exists());
    assert_eq!(files_in(directory.path()), ["catalog.json"]);
}

#[test]
fn replace_file_reports_a_staging_failure() {
    let directory = temp_dir();
    let target = directory.path().join("catalog.json");
    let files = ScriptedFiles::failing(FileFailure::Stage);
    let error = replace_file(&target, "body", &files).unwrap_err();
    assert_eq!(error.to_string(), "scripted filesystem failure");
    assert!(!target.exists());
    assert!(files.staged_paths().is_empty());
}

#[test]
fn replace_file_removes_the_staged_file_when_writing_fails() {
    let directory = temp_dir();
    let target = directory.path().join("catalog.json");
    let files = ScriptedFiles::failing(FileFailure::Write);
    let error = replace_file(&target, "body", &files).unwrap_err();
    assert_eq!(error.to_string(), "scripted filesystem failure");
    assert!(!target.exists());
    assert!(!files.staged_paths()[0].exists());
    assert!(files_in(directory.path()).is_empty());
}

#[test]
fn replace_file_removes_the_staged_file_when_replacing_fails() {
    let directory = temp_dir();
    let target = directory.path().join("catalog.json");
    let files = ScriptedFiles::failing(FileFailure::Replace);
    let error = replace_file(&target, "body", &files).unwrap_err();
    assert_eq!(error.to_string(), "scripted filesystem failure");
    assert!(!target.exists());
    assert!(!files.staged_paths()[0].exists());
    assert!(files_in(directory.path()).is_empty());
}

#[test]
fn codex_debug_models_returns_stdout_and_passes_the_environment() {
    let debug = ScriptedDebug::new(vec![DebugAnswer::Exited {
        code: 0_i32,
        stdout: "{\"models\":[]}".to_owned(),
        stderr: "ignored".to_owned(),
    }]);
    let env = vars(&[("HOME", "/home/tester")]);
    let outcome = codex_debug_models(CODEX_BINARY, &[], &env, &debug).unwrap();
    let DebugModelsOutcome::Printed(stdout) = outcome else {
        panic!("expected printed output");
    };
    assert_eq!(stdout, "{\"models\":[]}");
    let calls = debug.calls();
    assert_eq!(calls.len(), 1_usize);
    assert_eq!(
        calls[0].command,
        WindowsCommand::Direct(CODEX_BINARY.to_owned(), command(&["debug", "models"]))
    );
    assert_eq!(calls[0].env, env);
    assert_eq!(calls[0].timeout, CODEX_DEBUG_TIMEOUT);
    assert_eq!(calls[0].timeout, Duration::from_secs(10_u64));
}

#[test]
fn codex_debug_models_passes_leading_arguments_before_the_subcommand() {
    let debug = ScriptedDebug::new(vec![ScriptedDebug::printed("{}")]);
    let args = command(&["-c", "model_catalog_json=\"x\""]);
    let outcome = codex_debug_models(CODEX_BINARY, &args, &EnvVars::new(), &debug).unwrap();
    assert!(matches!(outcome, DebugModelsOutcome::Printed(_)));
    assert_eq!(
        debug.calls()[0].command,
        WindowsCommand::Direct(
            CODEX_BINARY.to_owned(),
            command(&["-c", "model_catalog_json=\"x\"", "debug", "models"])
        )
    );
}

#[test]
fn codex_debug_models_reports_the_first_stderr_line_on_failure() {
    let debug = ScriptedDebug::new(vec![ScriptedDebug::exited(
        2_i32,
        "error: unknown command `debug`\nmore detail\n",
    )]);
    let outcome = codex_debug_models(CODEX_BINARY, &[], &EnvVars::new(), &debug).unwrap();
    let DebugModelsOutcome::Skipped(skipped) = outcome else {
        panic!("expected a skipped debug probe");
    };
    assert_eq!(
        skipped.reason,
        "`codex debug models` exited 2: error: unknown command `debug`"
    );
}

#[test]
fn codex_debug_models_reports_no_output_when_stderr_is_blank() {
    let debug = ScriptedDebug::new(vec![ScriptedDebug::exited(3_i32, "  \n\t")]);
    let outcome = codex_debug_models(CODEX_BINARY, &[], &EnvVars::new(), &debug).unwrap();
    let DebugModelsOutcome::Skipped(skipped) = outcome else {
        panic!("expected a skipped debug probe");
    };
    assert_eq!(skipped.reason, "`codex debug models` exited 3: no output");
}

#[test]
fn codex_debug_models_reports_a_timeout() {
    let debug = ScriptedDebug::new(vec![DebugAnswer::Timeout]);
    let outcome = codex_debug_models(CODEX_BINARY, &[], &EnvVars::new(), &debug).unwrap();
    let DebugModelsOutcome::Skipped(skipped) = outcome else {
        panic!("expected a skipped debug probe");
    };
    assert_eq!(
        skipped.reason,
        "`codex debug models` timed out after 10 seconds"
    );
}

#[test]
fn codex_debug_models_reports_an_io_failure() {
    let debug = ScriptedDebug::new(vec![DebugAnswer::Io(io::ErrorKind::NotFound)]);
    let outcome = codex_debug_models(CODEX_BINARY, &[], &EnvVars::new(), &debug).unwrap();
    let DebugModelsOutcome::Skipped(skipped) = outcome else {
        panic!("expected a skipped debug probe");
    };
    assert!(skipped.reason.starts_with("`codex debug models` failed: "));
}

#[test]
fn codex_debug_models_reports_invalid_utf8() {
    let debug = ScriptedDebug::new(vec![DebugAnswer::Decode]);
    let outcome = codex_debug_models(CODEX_BINARY, &[], &EnvVars::new(), &debug).unwrap();
    let DebugModelsOutcome::Skipped(skipped) = outcome else {
        panic!("expected a skipped debug probe");
    };
    assert!(
        skipped
            .reason
            .starts_with("`codex debug models` printed output that is not valid UTF-8: ")
    );
}

#[test]
fn codex_debug_models_quotes_batch_shims_for_cmd() {
    let debug = ScriptedDebug::new(vec![ScriptedDebug::printed("{}")]);
    let outcome = codex_debug_models("/opt/codex.cmd", &[], &EnvVars::new(), &debug).unwrap();
    assert!(matches!(outcome, DebugModelsOutcome::Printed(_)));
    assert_eq!(
        debug.calls()[0].command,
        WindowsCommand::Batch(
            "/d /e:on /v:off /s /c \"\"/opt/codex.cmd\" \"debug\" \"models\"\"".to_owned()
        )
    );
}

#[test]
fn codex_debug_models_refuses_a_line_break_in_a_batch_argument() {
    let debug = ScriptedDebug::new(vec![ScriptedDebug::printed("{}")]);
    let args = command(&["-c", "bad\nvalue"]);
    let result = codex_debug_models("/opt/codex.cmd", &args, &EnvVars::new(), &debug);
    let Err(AgentRunError::Launch(LaunchError::LineBreak(name))) = result else {
        panic!("expected a line-break refusal");
    };
    assert_eq!(name, "codex.cmd");
    assert!(debug.calls().is_empty());
}

#[test]
fn stock_codex_models_parses_the_catalog() {
    let printed = stock_catalog(&[stock_row("gpt-known", 7_i64), stock_row("gpt-other", 9_i64)]);
    let debug = ScriptedDebug::new(vec![ScriptedDebug::printed(&printed)]);
    let parsed = stock_codex_models(CODEX_BINARY, &EnvVars::new(), &debug).unwrap();
    let StockModels::Models(models) = parsed else {
        panic!("expected a parsed stock catalog");
    };
    assert_eq!(models.len(), 2_usize);
    assert_eq!(models[0].slug, "gpt-known");
    assert_eq!(models[0].priority, 7_i64);
    assert_eq!(models[1].slug, "gpt-other");
    assert!(models[0].supported_in_api);
}

#[test]
fn stock_codex_models_reports_an_unparsable_catalog() {
    let debug = ScriptedDebug::new(vec![ScriptedDebug::printed("not json")]);
    let models = stock_codex_models(CODEX_BINARY, &EnvVars::new(), &debug).unwrap();
    let StockModels::Skipped(skipped) = models else {
        panic!("expected a skipped stock catalog");
    };
    assert!(
        skipped
            .reason
            .starts_with("`codex debug models` printed no model catalog: ")
    );
}

#[test]
fn stock_codex_models_reports_a_missing_models_key() {
    let debug = ScriptedDebug::new(vec![ScriptedDebug::printed("{\"entries\":[]}")]);
    let models = stock_codex_models(CODEX_BINARY, &EnvVars::new(), &debug).unwrap();
    let StockModels::Skipped(skipped) = models else {
        panic!("expected a skipped stock catalog");
    };
    assert!(skipped.reason.contains("missing field `models`"));
}

#[test]
fn stock_codex_models_propagates_a_debug_skip() {
    let debug = ScriptedDebug::new(vec![ScriptedDebug::exited(1_i32, "boom")]);
    let models = stock_codex_models(CODEX_BINARY, &EnvVars::new(), &debug).unwrap();
    let StockModels::Skipped(skipped) = models else {
        panic!("expected a skipped stock catalog");
    };
    assert_eq!(skipped.reason, "`codex debug models` exited 1: boom");
}
