#[cfg(unix)]
fn write_script(home: &TempDir, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let path = home.path().join(name);
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[tokio::test]
async fn codex_probe_reports_the_installed_version() {
    let home = TempDir::new().unwrap();
    let good = write_script(&home, "codex", "#!/bin/sh\nprintf 'codex-cli 0.129.0\\n'\n");
    let reported = installed_codex_version(&good, Duration::from_secs(5)).await;
    assert_eq!(reported.as_deref(), Some("codex-cli 0.129.0\n"));
    let failing = write_script(&home, "failing", "#!/bin/sh\nexit 3\n");
    assert!(
        installed_codex_version(&failing, Duration::from_secs(5))
            .await
            .is_none()
    );
    let slow = write_script(&home, "slow", "#!/bin/sh\nsleep 2\n");
    assert!(
        installed_codex_version(&slow, Duration::from_millis(100))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn unconfigure_without_receipt_reports_nothing_to_undo() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\n").unwrap();
    let error = unconfigure_at(&settings, &PrivateFiles::default())
        .await
        .unwrap_err();
    assert!(matches!(error, CodexSettingsError::NothingToUndo));
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        "model = \"original\"\n"
    );
}

#[tokio::test]
async fn repeat_configure_keeps_the_original_baseline() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\n").unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let edited = fs::read_to_string(&settings)
        .unwrap()
        .replace(&format!("model = \"{MODEL}\""), "model = \"user-edited\"");
    fs::write(&settings, edited).unwrap();
    configure_with(&settings, &files, GATEWAY, "sk-rotated", "third-model")
        .await
        .unwrap();
    unconfigure_at(&settings, &files).await.unwrap();
    let document = document_at(&settings);
    assert_eq!(
        string_at(&document, "model").as_deref(),
        Some("user-edited")
    );
}

#[tokio::test]
async fn unconfigure_keeps_an_edited_provider() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(
        &settings,
        "[model_providers.litellm]\nbase_url = \"https://old.example.com/v1\"\nhttp_headers = { Authorization = \"Bearer old-key\" }\n",
    )
    .unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let edited = fs::read_to_string(&settings)
        .unwrap()
        .replace(GATEWAY, "https://user.example.com");
    fs::write(&settings, edited).unwrap();
    let outcome = unconfigure_at(&settings, &files).await.unwrap();
    assert_eq!(outcome.kept, vec!["model_providers.litellm".to_owned()]);
    let document = document_at(&settings);
    assert_eq!(
        string_at(&document, "model_providers.litellm.base_url").as_deref(),
        Some("https://user.example.com/v1")
    );
    assert_eq!(
        string_at(
            &document,
            "model_providers.litellm.http_headers.Authorization"
        )
        .as_deref(),
        Some("Bearer sk-test")
    );
    assert!(!fs::read_to_string(&settings).unwrap().contains("old-key"));
}

#[tokio::test]
async fn unconfigure_restores_a_dotted_provider() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model_providers.litellm.name = \"old\"\n").unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    assert!(
        fs::read_to_string(&settings)
            .unwrap()
            .contains("Bearer sk-test")
    );
    let outcome = unconfigure_at(&settings, &files).await.unwrap();
    assert!(
        outcome
            .restored
            .contains(&"model_providers.litellm".to_owned())
    );
    let text = fs::read_to_string(&settings).unwrap();
    assert!(!text.contains("Authorization"));
    assert!(!text.contains("sk-test"));
    assert_eq!(
        string_at(&document_at(&settings), "model_providers.litellm.name").as_deref(),
        Some("old")
    );
}

#[tokio::test]
async fn configure_keeps_an_inline_providers_table() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(
        &settings,
        "model_providers = { other = { name = \"Keep\" } }\n",
    )
    .unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let configured = document_at(&settings);
    assert_eq!(
        string_at(&configured, "model_providers.other.name").as_deref(),
        Some("Keep")
    );
    assert_eq!(
        string_at(&configured, "model_providers.litellm.base_url").as_deref(),
        Some("https://gateway.example.com/team/v1")
    );
    unconfigure_at(&settings, &files).await.unwrap();
    let restored = document_at(&settings);
    assert_eq!(
        string_at(&restored, "model_providers.other.name").as_deref(),
        Some("Keep")
    );
    assert!(item_at(&restored, "model_providers.litellm").is_none());
}

#[tokio::test]
async fn configure_rejects_an_old_codex() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"keep\"\n").unwrap();
    let key = secret(KEY);
    let request = ConfigureCodexSettings {
        base_url: GATEWAY,
        api_key: &key,
        model: MODEL,
        settings_path: &settings,
        version: CodexVersion::Reported(Some("codex-cli 0.128.9")),
    };
    let error = configure_codex_settings(&request, &PrivateFiles::default())
        .await
        .unwrap_err();
    assert!(matches!(error, CodexSettingsError::UnsafeCodex));
    assert_eq!(fs::read_to_string(&settings).unwrap(), "model = \"keep\"\n");
    assert!(!codex_configure_state_path(&settings).unwrap().exists());
}
#[tokio::test]
async fn receipt_records_missing_profile_as_the_sentinel() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\nprofile = \"work\"\n").unwrap();
    configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let receipt = read_receipt(&settings).await.unwrap().unwrap();
    assert_eq!(
        receipt.written.get("profile").map(String::as_str),
        Some("ffa63583dfa6706b87d284b86b0d693a161e4840aad2c5cf6b5d27c3b9621f7d")
    );
    let saved = receipt
        .previous
        .get("profile")
        .and_then(|saved| saved.as_deref())
        .unwrap();
    assert_eq!(Snapshot::parse(saved).unwrap().canonical(), "\"work\"");
    let recorded: Vec<&str> = OWNED_PATHS
        .iter()
        .copied()
        .filter(|path| receipt.written.contains_key(*path))
        .collect();
    assert_eq!(
        recorded,
        vec![
            "model_provider",
            "model",
            "profile",
            "model_providers.litellm"
        ]
    );
}

#[tokio::test]
async fn receipt_rejects_a_mismatched_field_set() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let resolved = resolve_settings_path(&settings).unwrap();
    let quoted = serde_json::to_string(&settings_path_text(&resolved).unwrap()).unwrap();
    let receipt = format!(
        "{{\"version\":1,\"settings_path\":{quoted},\"file_existed\":false,\"providers_existed\":false,\"previous\":{{\"model\":null}},\"written\":{{}}}}"
    );
    fs::write(&state_path, receipt).unwrap();
    let error = read_receipt(&settings).await.unwrap_err();
    assert!(matches!(
        error,
        CodexSettingsError::Receipt {
            source: ReceiptSource::Keys,
            ..
        }
    ));
    let ownership = format!(
        "{{\"version\":1,\"settings_path\":{quoted},\"file_existed\":false,\"providers_existed\":false,\"previous\":{{\"other\":null}},\"written\":{{\"other\":\"{}\"}}}}",
        "0".repeat(64)
    );
    fs::write(&state_path, ownership).unwrap();
    let ownership_error = read_receipt(&settings).await.unwrap_err();
    assert!(matches!(
        ownership_error,
        CodexSettingsError::Receipt {
            source: ReceiptSource::Ownership,
            ..
        }
    ));
}

#[tokio::test]
async fn receipt_rejects_unknown_fields() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let resolved = resolve_settings_path(&settings).unwrap();
    let quoted = serde_json::to_string(&settings_path_text(&resolved).unwrap()).unwrap();
    let extra = format!(
        "{{\"version\":1,\"settings_path\":{quoted},\"file_existed\":false,\"providers_existed\":false,\"previous\":{{}},\"written\":{{}},\"surprise\":true}}"
    );
    fs::write(&state_path, extra).unwrap();
    let error = read_receipt(&settings).await.unwrap_err();
    assert!(matches!(
        error,
        CodexSettingsError::Receipt {
            source: ReceiptSource::Json(_),
            ..
        }
    ));
}

#[test]
fn with_value_leaves_a_non_table_section_alone() {
    let document = "model_providers = \"not a table\"\n"
        .parse::<DocumentMut>()
        .unwrap();
    let provider = Snapshot::parse("[value]\nname = \"gw\"\n").unwrap();
    let updated = with_value(&document, "model_providers.litellm", Some(&provider));
    assert_eq!(updated.to_string(), document.to_string());
    let removed = with_value(&document, "model_providers.litellm", None);
    assert_eq!(removed.to_string(), document.to_string());
}

#[tokio::test]
async fn receipt_round_trips_through_configure() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\nprofile = \"work\"\n").unwrap();
    configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let receipt: CodexReceipt = read_receipt(&settings).await.unwrap().unwrap();
    assert_eq!(receipt.version, 1);
    assert_eq!(receipt.previous.len(), receipt.written.len());
    assert!(receipt.file_existed);
    assert!(!receipt.providers_existed);
    let saved = receipt
        .previous
        .get("profile")
        .and_then(|saved| saved.as_deref())
        .unwrap();
    assert_eq!(Snapshot::parse(saved).unwrap().canonical(), "\"work\"");
}

#[cfg(unix)]
#[tokio::test]
async fn configure_stages_owner_only_files() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let state_path = codex_configure_state_path(&settings).unwrap();
    assert_eq!(directory_mode(state_path.parent().unwrap()), 0o700);
    assert_eq!(file_mode(&settings), 0o600);
    assert_eq!(file_mode(&state_path), 0o600);
}

#[cfg(unix)]
fn file_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;

    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
fn directory_mode(path: &Path) -> u32 {
    file_mode(path)
}

#[tokio::test]
async fn read_settings_reports_a_directory() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::create_dir_all(&settings).unwrap();
    let error = read_settings(&settings).await.unwrap_err();
    assert!(matches!(error, CodexSettingsError::Read { .. }));
}

#[tokio::test]
async fn read_settings_reports_non_utf8() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, [0xff_u8, 0xfe, 0x00]).unwrap();
    let error = read_settings(&settings).await.unwrap_err();
    assert!(matches!(error, CodexSettingsError::NotUtf8 { .. }));
}

#[tokio::test]
async fn read_receipt_reports_a_directory() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"keep\"\n").unwrap();
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(&state_path).unwrap();
    let error = read_receipt(&settings).await.unwrap_err();
    assert!(matches!(error, CodexSettingsError::Receipt { .. }));
}

#[tokio::test]
async fn receipt_accepts_a_missing_version() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let resolved = resolve_settings_path(&settings).unwrap();
    let quoted = serde_json::to_string(&settings_path_text(&resolved).unwrap()).unwrap();
    let receipt = format!(
        "{{\"settings_path\":{quoted},\"file_existed\":false,\"providers_existed\":false,\"previous\":{{}},\"written\":{{}}}}"
    );
    fs::write(&state_path, receipt).unwrap();
    let parsed = read_receipt(&settings).await.unwrap().unwrap();
    assert_eq!(parsed.version, 1);
}

#[tokio::test]
async fn receipt_rejects_a_multi_key_snapshot() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let resolved = resolve_settings_path(&settings).unwrap();
    let quoted = serde_json::to_string(&settings_path_text(&resolved).unwrap()).unwrap();
    let saved = serde_json::to_string("value = 1\nother = 2\n").unwrap();
    let receipt = format!(
        "{{\"version\":1,\"settings_path\":{quoted},\"file_existed\":false,\"providers_existed\":false,\"previous\":{{\"model\":{saved}}},\"written\":{{\"model\":\"{}\"}}}}",
        "0".repeat(64)
    );
    fs::write(&state_path, receipt).unwrap();
    let error = read_receipt(&settings).await.unwrap_err();
    assert!(matches!(
        error,
        CodexSettingsError::Receipt {
            source: ReceiptSource::Snapshot(_),
            ..
        }
    ));
}

#[tokio::test]
async fn repeated_configure_keeps_the_displaced_value() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\n").unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    configure_with(&settings, &files, GATEWAY, KEY, "second-model")
        .await
        .unwrap();
    unconfigure_at(&settings, &files).await.unwrap();
    let document = document_at(&settings);
    assert_eq!(string_at(&document, "model").as_deref(), Some("original"));
}

#[tokio::test]
async fn unconfigure_skips_a_field_restored_unchanged() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\n").unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    configure_with(&settings, &files, GATEWAY, KEY, "original")
        .await
        .unwrap();
    let outcome = unconfigure_at(&settings, &files).await.unwrap();
    assert!(!outcome.restored.contains(&"model".to_owned()));
    let document = document_at(&settings);
    assert_eq!(string_at(&document, "model").as_deref(), Some("original"));
}

#[tokio::test]
async fn unconfigure_after_the_user_deleted_the_file() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\nprofile = \"work\"\n").unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    fs::remove_file(&settings).unwrap();
    let outcome = unconfigure_at(&settings, &files).await.unwrap();
    assert!(outcome.file_removed);
    assert!(outcome.restored.is_empty());
    assert!(!settings.exists());
}

#[tokio::test]
async fn configure_round_trips_an_api_key_with_toml_specials() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let api_key = "sk-\"quoted\"\\slash\nnext-line";
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, api_key, MODEL)
        .await
        .unwrap();
    let document = document_at(&settings);
    assert_eq!(
        string_at(
            &document,
            "model_providers.litellm.http_headers.Authorization"
        ),
        Some(format!("Bearer {api_key}"))
    );
    unconfigure_at(&settings, &files).await.unwrap();
    assert!(!settings.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn configure_through_a_symlink_updates_the_target() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new().unwrap();
    let target = home.path().join("real-config.toml");
    fs::write(&target, "model = \"old\"\n").unwrap();
    let alias = home.path().join("config.toml");
    symlink(&target, &alias).unwrap();
    let files = PrivateFiles::default();
    configure_with(&alias, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    assert!(alias.is_symlink());
    assert!(
        fs::read_to_string(&target)
            .unwrap()
            .contains("Bearer sk-test")
    );
    assert_eq!(
        codex_configure_state_path(&alias).unwrap(),
        codex_configure_state_path(&target).unwrap()
    );
    unconfigure_at(&alias, &files).await.unwrap();
    assert!(alias.is_symlink());
    let document = document_at(&alias);
    assert_eq!(string_at(&document, "model").as_deref(), Some("old"));
}

#[cfg(unix)]
#[tokio::test]
async fn configure_refuses_a_non_utf8_settings_path() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let home = TempDir::new().unwrap();
    let settings = home
        .path()
        .join(OsString::from_vec(b"config-\xff.toml".to_vec()));
    let error = configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap_err();
    assert!(matches!(error, CodexSettingsError::NonUtf8Path { .. }));
    assert!(!settings.exists());
    assert!(!codex_configure_state_path(&settings).unwrap().exists());
}

#[tokio::test]
async fn configure_receipt_failure_discards_both_staged_files() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"old\"\n").unwrap();
    let (files, journal) = journal_files();
    let state_path = codex_configure_state_path(&settings).unwrap();
    journal.fail_replace(&state_path, 1);
    let error = configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap_err();
    assert!(matches!(error, CodexSettingsError::WriteReceipt { .. }));
    assert_eq!(fs::read_to_string(&settings).unwrap(), "model = \"old\"\n");
    assert!(!state_path.exists());
    let resolved_parent = resolve_settings_path(&settings).unwrap();
    let parent = resolved_parent.parent().unwrap();
    let state_directory = parent.join(".litellm");
    assert!(staging_files(&[parent, state_directory.as_path()]).is_empty());
}

#[tokio::test]
async fn unconfigure_keeps_the_receipt_when_the_write_fails() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"old\"\n").unwrap();
    configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let before = fs::read(&settings).unwrap();
    let state_path = codex_configure_state_path(&settings).unwrap();
    let (files, journal) = journal_files();
    journal.fail_replace(&resolve_settings_path(&settings).unwrap(), 1);
    let error = unconfigure_at(&settings, &files).await.unwrap_err();
    assert!(matches!(error, CodexSettingsError::Undo { .. }));
    assert_eq!(fs::read(&settings).unwrap(), before);
    assert!(state_path.exists());
    unconfigure_at(&settings, &PrivateFiles::default())
        .await
        .unwrap();
    let document = document_at(&settings);
    assert_eq!(string_at(&document, "model").as_deref(), Some("old"));
}
