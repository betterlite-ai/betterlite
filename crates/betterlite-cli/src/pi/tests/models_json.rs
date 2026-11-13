#[test]
fn json_path_uses_the_pi_coding_agent_dir() {
    let directory = TempDir::new().unwrap();
    let environment = FakeEnvironment {
        variables: BTreeMap::from([(
            "PI_CODING_AGENT_DIR".to_owned(),
            directory.path().to_str().unwrap().to_owned(),
        )]),
        home: None,
    };
    assert_eq!(PI_CONFIG_DIR_ENV, "PI_CODING_AGENT_DIR");
    assert_eq!(
        models_json_path(&environment),
        Some(directory.path().join("models.json"))
    );
}

#[test]
fn json_path_prefers_pi_coding_agent_dir_over_the_home_directory() {
    let directory = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment {
        variables: BTreeMap::from([(
            PI_CONFIG_DIR_ENV.to_owned(),
            directory.path().to_str().unwrap().to_owned(),
        )]),
        home: Some(home.path().to_owned()),
    };
    assert_eq!(
        models_json_path(&environment),
        Some(directory.path().join("models.json"))
    );
}

#[test]
fn json_path_ignores_an_empty_pi_coding_agent_dir() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment {
        variables: BTreeMap::from([(PI_CONFIG_DIR_ENV.to_owned(), String::new())]),
        home: Some(home.path().to_owned()),
    };
    assert_eq!(
        models_json_path(&environment),
        Some(home.path().join(".pi").join("agent").join("models.json"))
    );
}

#[test]
fn json_path_defaults_to_the_home_agent_directory() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    assert_eq!(
        models_json_path(&environment),
        Some(home.path().join(".pi").join("agent").join("models.json"))
    );
}

#[test]
fn json_path_has_none_without_a_home_or_override() {
    assert_eq!(models_json_path(&FakeEnvironment::without_home()), None);
}

#[test]
fn json_path_sync_rejects_a_providers_value_that_is_not_an_object() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    fs::write(&path, r#"{"providers":[]}"#).unwrap();
    let error = sync_models_json(&path, BASE_URL, &[], &BTreeMap::new(), &LocalFiles).unwrap_err();
    assert_eq!(error.kind(), None);
    assert_eq!(error.status(), None);
    assert!(matches!(error, PiSyncError::ParseModels { .. }));
    assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"providers":[]}"#);
}

#[test]
fn json_path_sync_rejects_a_non_utf8_file() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    fs::write(&path, [0xff_u8, 0xfe]).unwrap();
    let error = sync_models_json(&path, BASE_URL, &[], &BTreeMap::new(), &LocalFiles).unwrap_err();
    assert!(matches!(error, PiSyncError::ParseModels { .. }));
    assert_eq!(fs::read(&path).unwrap(), vec![0xff, 0xfe]);
}

#[rstest]
#[case::empty("")]
#[case::blank("  \n")]
#[case::invalid_json("{")]
#[case::array("[]")]
#[case::string("\"models\"")]
#[case::number("42")]
#[case::null("null")]
#[case::providers_null(r#"{"providers":null}"#)]
#[case::providers_string(r#"{"providers":"pi"}"#)]
fn json_path_sync_rejects_an_invalid_models_file(#[case] text: &str) {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    fs::write(&path, text).unwrap();
    let error = sync_models_json(&path, BASE_URL, &[], &BTreeMap::new(), &LocalFiles).unwrap_err();
    assert_eq!(error.kind(), None);
    assert!(matches!(error, PiSyncError::ParseModels { .. }));
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn provider_block_has_the_openai_completions_shape() {
    let block = provider_block(
        "https://proxy.test/",
        &["gpt-4o".to_owned()],
        &BTreeMap::new(),
    );
    assert_eq!(block.base_url, "https://proxy.test/v1");
    assert_eq!(block.api, "openai-completions");
    assert_eq!(block.api_key_reference, "$LITELLM_PROXY_API_KEY");
    assert_eq!(
        block.models,
        vec![ModelEntry {
            id: "gpt-4o".to_owned(),
            context_window: None,
            max_tokens: None,
        }]
    );
}

#[test]
fn provider_block_uses_the_shared_key_environment_constant() {
    let block = provider_block(BASE_URL, &[], &BTreeMap::new());
    assert_eq!(
        block.api_key_reference,
        format!("${LITELLM_PROXY_API_KEY_ENV}")
    );
}

#[test]
fn provider_block_strips_trailing_slashes_from_the_base_url() {
    let block = provider_block("https://proxy.test///", &[], &BTreeMap::new());
    assert_eq!(block.base_url, "https://proxy.test/v1");
}

#[test]
fn provider_block_includes_model_limits() {
    let limits = BTreeMap::from([(
        "a".to_owned(),
        ModelLimits {
            context_window: Some(128_000),
            max_tokens: Some(4096),
        },
    )]);
    let block = provider_block(BASE_URL, &["a".to_owned(), "b".to_owned()], &limits);
    assert_eq!(block.models[0].context_window, Some(128_000));
    assert_eq!(block.models[0].max_tokens, Some(4096));
    assert_eq!(block.models[1].context_window, None);
    assert_eq!(block.models[1].max_tokens, None);
}

#[test]
fn provider_block_serializes_with_pi_field_names() {
    let limits = BTreeMap::from([(
        "a".to_owned(),
        ModelLimits {
            context_window: Some(128_000),
            max_tokens: Some(4096),
        },
    )]);
    let block = provider_block(BASE_URL, &["a".to_owned(), "b".to_owned()], &limits);
    let value = serde_json::to_value(&block).unwrap();
    assert_eq!(
        value.pointer("/baseUrl").unwrap().as_str(),
        Some("https://proxy.test/v1")
    );
    assert_eq!(
        value.pointer("/api").unwrap().as_str(),
        Some("openai-completions")
    );
    assert_eq!(
        value.pointer("/apiKey").unwrap().as_str(),
        Some("$LITELLM_PROXY_API_KEY")
    );
    assert_eq!(value.pointer("/models/0/id").unwrap().as_str(), Some("a"));
    assert_eq!(
        value.pointer("/models/0/contextWindow").unwrap().as_i64(),
        Some(128_000)
    );
    assert_eq!(
        value.pointer("/models/0/maxTokens").unwrap().as_i64(),
        Some(4096)
    );
    assert_eq!(value.pointer("/models/1/id").unwrap().as_str(), Some("b"));
    let bare_entry = value.pointer("/models/1").unwrap();
    assert!(bare_entry.get("contextWindow").is_none());
    assert!(bare_entry.get("maxTokens").is_none());
}

#[test]
fn atomic_write_creates_a_missing_file_and_its_directory() {
    let directory = TempDir::new().unwrap();
    let path = directory
        .path()
        .join("nested")
        .join("agent")
        .join("models.json");
    sync_models_json(
        &path,
        BASE_URL,
        &["a".to_owned()],
        &BTreeMap::new(),
        &LocalFiles,
    )
    .unwrap();
    let document = read_json(&path);
    assert_eq!(
        document.pointer("/providers/litellm/api").unwrap().as_str(),
        Some("openai-completions")
    );
}

#[test]
fn atomic_write_preserves_other_providers_and_settings() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    fs::write(
        &path,
        r#"{"theme":"dark","providers":{"other":{"baseUrl":"https://other.test"}}}"#,
    )
    .unwrap();
    sync_models_json(
        &path,
        BASE_URL,
        &["a".to_owned()],
        &BTreeMap::new(),
        &LocalFiles,
    )
    .unwrap();
    let document = read_json(&path);
    assert_eq!(document.pointer("/theme").unwrap().as_str(), Some("dark"));
    assert_eq!(
        document
            .pointer("/providers/other/baseUrl")
            .unwrap()
            .as_str(),
        Some("https://other.test")
    );
    assert_eq!(
        document
            .pointer("/providers/litellm/baseUrl")
            .unwrap()
            .as_str(),
        Some("https://proxy.test/v1")
    );
}

#[test]
fn atomic_write_adds_providers_to_a_file_without_one() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    fs::write(&path, r#"{"theme":"dark"}"#).unwrap();
    sync_models_json(
        &path,
        BASE_URL,
        &["a".to_owned()],
        &BTreeMap::new(),
        &LocalFiles,
    )
    .unwrap();
    let document = read_json(&path);
    assert_eq!(document.pointer("/theme").unwrap().as_str(), Some("dark"));
    assert_eq!(
        document
            .pointer("/providers/litellm/baseUrl")
            .unwrap()
            .as_str(),
        Some("https://proxy.test/v1")
    );
}

#[test]
fn atomic_write_replaces_an_existing_litellm_provider() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    fs::write(
        &path,
        r#"{"providers":{"litellm":{"baseUrl":"https://old.test/v1","api":"legacy","headers":{"x":"y"}},"other":{}}}"#,
    )
    .unwrap();
    sync_models_json(
        &path,
        BASE_URL,
        &["a".to_owned()],
        &BTreeMap::new(),
        &LocalFiles,
    )
    .unwrap();
    let document = read_json(&path);
    assert_eq!(
        document
            .pointer("/providers/litellm/baseUrl")
            .unwrap()
            .as_str(),
        Some("https://proxy.test/v1")
    );
    assert_eq!(
        document.pointer("/providers/litellm/api").unwrap().as_str(),
        Some("openai-completions")
    );
    // A merge into the old entry would keep this stale key and pi would keep sending it
    assert!(document.pointer("/providers/litellm/headers").is_none());
    assert!(document.pointer("/providers/other").is_some());
}

#[test]
fn atomic_write_ends_the_file_with_a_newline() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    sync_models_json(&path, BASE_URL, &[], &BTreeMap::new(), &LocalFiles).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.ends_with('\n'));
    let document = read_json(&path);
    assert!(document.pointer("/providers/litellm").is_some());
}

#[test]
fn atomic_write_stages_in_the_target_directory_and_renames() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    let files = ScriptedFiles::new(FileFault::None);
    sync_models_json(&path, BASE_URL, &[], &BTreeMap::new(), &files).unwrap();
    let calls = files.calls();
    assert_eq!(
        calls[0],
        FileCall::CreateParents(directory.path().to_owned())
    );
    let FileCall::CreateStaged {
        directory: staged_directory,
        suffix,
    } = &calls[1]
    else {
        panic!("no staged file was created");
    };
    assert_eq!(staged_directory, directory.path());
    assert_eq!(suffix.as_deref(), Some(".tmp"));
    let FileCall::Write(staged) = &calls[2] else {
        panic!("the staged file was not written");
    };
    assert_eq!(staged.parent(), Some(directory.path()));
    let FileCall::Replace {
        staged: renamed,
        target,
    } = &calls[3]
    else {
        panic!("the staged file was not renamed");
    };
    assert_eq!(renamed, staged);
    assert_eq!(target, &path);
    assert_eq!(calls.len(), 4);
    assert!(path.exists());
}

#[test]
fn atomic_write_reports_a_failed_directory_creation() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    let files = ScriptedFiles::new(FileFault::CreateParents);
    let error = sync_models_json(&path, BASE_URL, &[], &BTreeMap::new(), &files).unwrap_err();
    assert!(matches!(error, PiSyncError::WriteModels { .. }));
    assert_eq!(
        files.calls(),
        vec![FileCall::CreateParents(directory.path().to_owned())]
    );
    assert!(!path.exists());
}

#[test]
fn atomic_write_reports_a_failed_stage_creation() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    let files = ScriptedFiles::new(FileFault::CreateStaged);
    let error = sync_models_json(&path, BASE_URL, &[], &BTreeMap::new(), &files).unwrap_err();
    assert!(matches!(error, PiSyncError::WriteModels { .. }));
    assert!(
        !files
            .calls()
            .iter()
            .any(|call| matches!(call, FileCall::Remove(_)))
    );
    assert!(!path.exists());
}

#[test]
fn atomic_write_removes_the_staged_file_when_the_write_fails() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    let files = ScriptedFiles::new(FileFault::Write);
    let error = sync_models_json(&path, BASE_URL, &[], &BTreeMap::new(), &files).unwrap_err();
    assert!(matches!(error, PiSyncError::WriteModels { .. }));
    let calls = files.calls();
    let Some(FileCall::Write(staged)) = calls.get(2).cloned() else {
        panic!("the staged file was not written");
    };
    assert!(calls.contains(&FileCall::Remove(staged.clone())));
    assert!(!staged.exists());
    assert!(!path.exists());
}

#[test]
fn atomic_write_removes_the_staged_file_when_the_rename_fails() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    let files = ScriptedFiles::new(FileFault::Replace);
    let error = sync_models_json(&path, BASE_URL, &[], &BTreeMap::new(), &files).unwrap_err();
    assert!(matches!(error, PiSyncError::WriteModels { .. }));
    let calls = files.calls();
    assert!(calls.iter().any(|call| matches!(call, FileCall::Remove(_))));
    assert!(!path.exists());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn atomic_write_leaves_a_directory_alone_when_reading_fails() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("models.json");
    fs::create_dir_all(&path).unwrap();
    let files = ScriptedFiles::new(FileFault::None);
    let error = sync_models_json(&path, BASE_URL, &[], &BTreeMap::new(), &files).unwrap_err();
    assert_eq!(error.kind(), None);
    assert!(matches!(error, PiSyncError::ReadModels { .. }));
    assert!(path.is_dir());
    assert!(files.calls().is_empty());
}
