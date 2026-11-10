#![cfg(test)]

use std::ffi::OsStr;
use std::fs;
use std::time::Duration;

use betterlite_logging::private_json::files::PrivateFiles;
use rstest::rstest;
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;
use toml_edit::{DocumentMut, Item, Value};

use crate::codex_settings::CODEX_PROXY_PROVIDER;
use crate::codex_settings::error::CodexSettingsError;
use crate::codex_settings::paths::{
    codex_configure_state_path, resolve_settings_path, settings_path_text,
};
use crate::codex_settings::preflight::preflight_codex_settings;
use crate::codex_settings::receipt::read_receipt;
use crate::codex_settings::snapshot::{Snapshot, fingerprint, snapshot, with_value};
use crate::codex_settings::version::{CodexVersion, installed_codex_version, require_safe_codex};

pub(crate) mod support;

use support::{
    GATEWAY, KEY, MODEL, SAFE_CODEX, configure_with, document_at, item_at, journal_files,
    staging_files, string_at, unconfigure_at,
};

#[test]
fn receipt_path_deterministic() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let first = codex_configure_state_path(&settings).unwrap();
    let second = codex_configure_state_path(&settings).unwrap();
    let other = codex_configure_state_path(&home.path().join("other.toml")).unwrap();
    assert_eq!(first, second);
    assert_ne!(first, other);
    let resolved = resolve_settings_path(&settings).unwrap();
    assert_eq!(
        first.parent().unwrap(),
        resolved.parent().unwrap().join(".litellm").as_path()
    );
    let name = first.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("codex_configure_"));
    assert_eq!(first.extension().and_then(OsStr::to_str), Some("json"));
}

#[test]
fn snapshot_missing_sentinel() {
    let sentinel = hex::encode(Sha256::digest(b"missing"));
    assert_eq!(fingerprint(None), sentinel);
    assert_eq!(
        fingerprint(None),
        "ffa63583dfa6706b87d284b86b0d693a161e4840aad2c5cf6b5d27c3b9621f7d"
    );
    let named = Snapshot::from_item(&Item::Value(Value::from("missing")));
    assert_ne!(fingerprint(Some(&named)), sentinel);
}

#[test]
fn fingerprint_determinism() {
    let first = fingerprint(Some(&Snapshot::parse("value = \"litellm\"\n").unwrap()));
    let spaced = fingerprint(Some(&Snapshot::parse("value   =   \"litellm\"\n").unwrap()));
    assert_eq!(first, spaced);
    assert_eq!(
        first,
        "e3eb10f5fe40a4d0cfd50f8afb78a13a5033c41d267d7399314ea83705faf90e"
    );
    let accented = Snapshot::parse("value = \"\u{e9}\u{e8}\"\n").unwrap();
    assert_eq!(
        fingerprint(Some(&accented)),
        "b1a5634e4d63d6dffa4cbec7a5ee5518970a953d3d06ca671d377a06d1e6f347"
    );
    let provider = Snapshot::parse("value = { name = \"old\" }\n").unwrap();
    assert_eq!(
        fingerprint(Some(&provider)),
        "9a41b5a0bdb4c0e92140fa2a04cc160640d083d93f3d5da9321a2e3b2f60fcb3"
    );
}

#[test]
fn snapshot_extracted_correct() {
    let document = "# head\nmodel = \"old\" # comment\n[model_providers.litellm]\nname = \"gw\"\n"
        .parse::<DocumentMut>()
        .unwrap();
    let model = snapshot(&document, "model").unwrap();
    assert_eq!(
        fingerprint(Some(&model)),
        fingerprint(Some(&Snapshot::parse("value = \"old\"\n").unwrap()))
    );
    let provider = snapshot(&document, "model_providers.litellm").unwrap();
    assert_eq!(
        fingerprint(Some(&provider)),
        fingerprint(Some(
            &Snapshot::parse("value = { name = \"gw\" }\n").unwrap()
        ))
    );
    let text: &str = provider.as_ref();
    let reparsed = text.parse::<DocumentMut>().unwrap();
    let name = reparsed
        .get("value")
        .and_then(Item::as_table)
        .and_then(|table| table.get("name"))
        .and_then(Item::as_str);
    assert_eq!(name, Some("gw"));
}

#[test]
fn snapshot_restored_modified() {
    let document = "# user\nmodel = \"old\" # keep\n[model_providers.other]\nname = \"Keep\"\n"
        .parse::<DocumentMut>()
        .unwrap();
    let replacement = Snapshot::from_item(&Item::Value(Value::from("new")));
    let updated = with_value(&document, "model", Some(&replacement));
    let text = updated.to_string();
    assert!(text.contains("# user"));
    assert!(text.contains("# keep"));
    assert_eq!(string_at(&updated, "model").as_deref(), Some("new"));
    assert_eq!(string_at(&document, "model").as_deref(), Some("old"));
    let removed = with_value(&updated, "model", None);
    assert!(item_at(&removed, "model").is_none());
    let provider = Snapshot::parse("[value]\nname = \"gw\"\n").unwrap();
    let appended = with_value(&removed, "model_providers.litellm", Some(&provider));
    assert_eq!(
        string_at(&appended, "model_providers.other.name").as_deref(),
        Some("Keep")
    );
    assert_eq!(
        string_at(&appended, "model_providers.litellm.name").as_deref(),
        Some("gw")
    );
}

#[test]
fn snapshot_missing_returns_none() {
    let document = "model = \"x\"\n[model_providers]\n"
        .parse::<DocumentMut>()
        .unwrap();
    assert!(snapshot(&document, "profile").is_none());
    assert!(snapshot(&document, "model_providers.litellm").is_none());
    assert!(snapshot(&document, "model_providers.other.name").is_none());
}

#[rstest]
#[case::minimum("codex-cli 0.129.0")]
#[case::patch("codex-cli 0.129.1")]
#[case::minor("codex-cli 0.130.0")]
#[case::major("codex-cli 1.0.0")]
#[case::padded(" \ncodex-cli 0.129.0\n")]
fn version_gate_met(#[case] version: &str) {
    require_safe_codex(Some(version)).unwrap();
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
#[case::older_minor(Some("codex-cli 0.99.0"))]
#[case::older_patch(Some("codex-cli 0.128.99"))]
#[case::prerelease_below(Some("codex-cli 0.129.0-alpha.1"))]
#[case::prerelease_above(Some("codex-cli 1.0.0-beta.1"))]
#[case::no_prefix(Some("0.129.0"))]
#[case::trailing_text(Some("codex-cli 0.129.0 extra"))]
fn version_gate_old_rejected(#[case] version: Option<&str>) {
    let error = require_safe_codex(version).unwrap_err();
    assert!(error.to_string().contains("0.129.0"));
    assert!(matches!(error, CodexSettingsError::UnsafeCodex));
}

#[tokio::test]
async fn version_not_installed() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let missing = preflight_codex_settings(&settings, CodexVersion::Reported(None)).await;
    assert!(matches!(missing, Err(CodexSettingsError::UnsafeCodex)));
    let absent = home.path().join("no-codex");
    assert!(
        installed_codex_version(&absent, Duration::from_millis(100))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn configure_receipt_staged_first() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"old\"\n").unwrap();
    let (files, journal) = journal_files();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let resolved = resolve_settings_path(&settings).unwrap();
    let staged_home = resolved.parent().unwrap().to_owned();
    let state_dir = staged_home.join(".litellm");
    assert_eq!(journal.staged_directories(), vec![state_dir, staged_home]);
    let state_path = codex_configure_state_path(&settings).unwrap();
    assert_eq!(journal.committed_paths(), vec![state_path, resolved]);
}

#[tokio::test]
async fn configure_settings_failure_discards_settings_keeps_receipt() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"old\"\n").unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let before = fs::read(&settings).unwrap();
    let state_path = codex_configure_state_path(&settings).unwrap();
    let receipt_before = fs::read(&state_path).unwrap();
    let (failing, journal) = journal_files();
    let target = resolve_settings_path(&settings).unwrap();
    journal.fail_replace(&target, 1);
    let error = configure_with(&settings, &failing, GATEWAY, "sk-replacement", "new-model")
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("sk-replacement"));
    assert!(matches!(error, CodexSettingsError::WriteSettings { .. }));
    assert_eq!(fs::read(&settings).unwrap(), before);
    assert_eq!(fs::read(&state_path).unwrap(), receipt_before);
    let resolved_parent = target.parent().unwrap();
    let state_directory = resolved_parent.join(".litellm");
    assert!(staging_files(&[resolved_parent, state_directory.as_path()]).is_empty());
}

#[tokio::test]
async fn configure_settings_rollback_restores_receipt() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"old\"\n").unwrap();
    configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let state_path = codex_configure_state_path(&settings).unwrap();
    let receipt_before = fs::read(&state_path).unwrap();
    let target = resolve_settings_path(&settings).unwrap();
    let (failing, journal) = journal_files();
    journal.fail_replace(&target, 1);
    let error = configure_with(&settings, &failing, GATEWAY, "sk-other", "other")
        .await
        .unwrap_err();
    assert!(matches!(error, CodexSettingsError::WriteSettings { .. }));
    assert_eq!(fs::read(&state_path).unwrap(), receipt_before);
    let (blocked, rollback) = journal_files();
    rollback.fail_replace(&target, 1);
    rollback.fail_replace(&state_path, 2);
    let halted = configure_with(&settings, &blocked, GATEWAY, "sk-third", "third")
        .await
        .unwrap_err();
    assert!(!halted.to_string().contains("sk-third"));
    assert!(matches!(halted, CodexSettingsError::RestoreReceipt { .. }));
}

fn assert_provider_table(document: &DocumentMut) {
    let provider = item_at(document, "model_providers.litellm")
        .unwrap()
        .as_table()
        .unwrap();
    let fields = (
        provider.get("name").and_then(Item::as_str),
        provider.get("base_url").and_then(Item::as_str),
        provider.get("wire_api").and_then(Item::as_str),
        provider.get("supports_websockets").and_then(Item::as_bool),
        provider.get("requires_openai_auth").and_then(Item::as_bool),
    );
    assert_eq!(
        fields,
        (
            Some("LiteLLM proxy"),
            Some("https://gateway.example.com/team/v1"),
            Some("responses"),
            Some(false),
            Some(false)
        )
    );
    let headers = provider.get("http_headers").and_then(Item::as_table);
    assert_eq!(
        headers
            .and_then(|table| table.get("Authorization"))
            .and_then(Item::as_str),
        Some("Bearer sk-test")
    );
}

#[tokio::test]
async fn configure_builds_authorization_header() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    assert_provider_table(&document_at(&settings));
    let receipt = read_receipt(&settings).await.unwrap().unwrap();
    assert_eq!(
        receipt
            .written
            .get("model_providers.litellm")
            .map(String::as_str),
        Some("059eb6a7bacdb79e8b1b97735eca95370b1ffcafe0108ea5007b88af74a5bbc8")
    );
    assert_eq!(
        receipt.written.get("model").map(String::as_str),
        Some("94c432c6c6ed3a612386883f88eb565dcc4f94363ab6bfcf7765b10956e5f813")
    );
    assert!(
        !fs::read_to_string(codex_configure_state_path(&settings).unwrap())
            .unwrap()
            .contains(KEY)
    );
}

#[tokio::test]
async fn configure_selects_model_and_provider() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(
        &settings,
        "# user settings\nmodel = \"old\" # starting model\nprofile = \"work\"\n[profiles.work]\nmodel = \"work-model\"\n[model_providers.other]\nname = \"Keep me\" # other provider\n",
    )
    .unwrap();
    configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let document = document_at(&settings);
    assert_eq!(string_at(&document, "model").as_deref(), Some(MODEL));
    assert_eq!(
        string_at(&document, "model_provider").as_deref(),
        Some(CODEX_PROXY_PROVIDER)
    );
    assert!(item_at(&document, "profile").is_none());
    assert_eq!(
        string_at(&document, "profiles.work.model").as_deref(),
        Some("work-model")
    );
    assert_eq!(
        string_at(&document, "model_providers.other.name").as_deref(),
        Some("Keep me")
    );
    let text = fs::read_to_string(&settings).unwrap();
    assert!(text.contains("# user settings"));
    assert!(text.contains("# starting model"));
    assert!(text.contains("# other provider"));
    let receipt = read_receipt(&settings).await.unwrap().unwrap();
    assert!(
        !fs::read_to_string(codex_configure_state_path(&settings).unwrap())
            .unwrap()
            .contains(KEY)
    );
    assert!(receipt.written.contains_key("model_providers.litellm"));
}

#[tokio::test]
async fn unconfigure_restored_classification() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(
        &settings,
        "model = \"original\"\nmodel_provider = \"openai\"\nprofile = \"work\"\n",
    )
    .unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let outcome = unconfigure_at(&settings, &files).await.unwrap();
    assert_eq!(
        outcome.restored,
        vec![
            "model_provider".to_owned(),
            "model".to_owned(),
            "profile".to_owned(),
            "model_providers.litellm".to_owned()
        ]
    );
    assert!(outcome.kept.is_empty());
    assert!(!outcome.file_removed);
    let document = document_at(&settings);
    assert_eq!(string_at(&document, "model").as_deref(), Some("original"));
    assert_eq!(
        string_at(&document, "model_provider").as_deref(),
        Some("openai")
    );
    assert_eq!(string_at(&document, "profile").as_deref(), Some("work"));
    assert!(item_at(&document, "model_providers.litellm").is_none());
}

#[tokio::test]
async fn unconfigure_kept_classification() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(
        &settings,
        "model = \"original\"\nmodel_provider = \"openai\"\n",
    )
    .unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let edited = fs::read_to_string(&settings).unwrap().replace(
        &format!("model = \"{MODEL}\""),
        "model = \"my-custom-model\"",
    );
    fs::write(&settings, edited).unwrap();
    let outcome = unconfigure_at(&settings, &files).await.unwrap();
    assert_eq!(outcome.kept, vec!["model".to_owned()]);
    assert_eq!(
        outcome.restored,
        vec![
            "model_provider".to_owned(),
            "model_providers.litellm".to_owned()
        ]
    );
    let document = document_at(&settings);
    assert_eq!(
        string_at(&document, "model").as_deref(),
        Some("my-custom-model")
    );
    assert_eq!(
        string_at(&document, "model_provider").as_deref(),
        Some("openai")
    );
}

#[rstest]
#[case::missing(None, true)]
#[case::empty(Some(""), false)]
#[case::comment(Some("# my preferences\n"), false)]
#[case::providers_table(Some("[model_providers]\n"), false)]
#[tokio::test]
async fn unconfigure_file_removed_classification(
    #[case] original: Option<&str>,
    #[case] expected_removed: bool,
) {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    if let Some(text) = original {
        fs::write(&settings, text).unwrap();
    }
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let outcome = unconfigure_at(&settings, &files).await.unwrap();
    assert_eq!(outcome.file_removed, expected_removed);
    assert_eq!(settings.exists(), original.is_some());
    if let Some(text) = original {
        let restored = fs::read_to_string(&settings).unwrap();
        assert_eq!(
            restored.contains("# my preferences"),
            text.contains("# my preferences")
        );
        assert_eq!(
            restored.contains("[model_providers]"),
            text.contains("[model_providers]")
        );
    }
}

#[tokio::test]
async fn preflight_codex_validates_version() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"keep\"\n").unwrap();
    let missing = preflight_codex_settings(&settings, CodexVersion::Reported(None)).await;
    assert!(matches!(missing, Err(CodexSettingsError::UnsafeCodex)));
    let old =
        preflight_codex_settings(&settings, CodexVersion::Reported(Some("codex-cli 0.128.9")))
            .await;
    assert!(matches!(old, Err(CodexSettingsError::UnsafeCodex)));
    preflight_codex_settings(&settings, CodexVersion::Reported(Some(SAFE_CODEX)))
        .await
        .unwrap();
    assert_eq!(fs::read_to_string(&settings).unwrap(), "model = \"keep\"\n");
}

#[rstest]
#[case::invalid_toml("model = \"unterminated\n")]
#[case::providers_scalar("model_providers = \"sk-secret\"\n")]
#[case::provider_scalar("[model_providers]\nlitellm = \"sk-secret\"\n")]
#[case::providers_array("model_providers = []\n")]
#[tokio::test]
async fn preflight_codex_validates_toml(#[case] text: &str) {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, text).unwrap();
    let error = preflight_codex_settings(&settings, CodexVersion::Reported(Some(SAFE_CODEX)))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CodexSettingsError::Parse { .. }
            | CodexSettingsError::ProvidersNotTable
            | CodexSettingsError::ProviderNotTable
    ));
    assert!(!error.to_string().contains("sk-secret"));
    assert_eq!(fs::read_to_string(&settings).unwrap(), text);
}

#[tokio::test]
async fn preflight_codex_validates_receipt() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"keep\"\n").unwrap();
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    fs::write(&state_path, "{\"previous\": \"sk-secret\"}").unwrap();
    let error = preflight_codex_settings(&settings, CodexVersion::Reported(Some(SAFE_CODEX)))
        .await
        .unwrap_err();
    assert!(matches!(error, CodexSettingsError::Receipt { .. }));
    assert!(!error.to_string().contains("sk-secret"));
    let resolved = resolve_settings_path(&settings).unwrap();
    let quoted = serde_json::to_string(&settings_path_text(&resolved).unwrap()).unwrap();
    fs::write(
        &state_path,
        format!(
            "{{\"version\":1,\"settings_path\":{quoted},\"file_existed\":true,\"providers_existed\":false,\"previous\":{{}},\"written\":{{}}}}"
        ),
    )
    .unwrap();
    preflight_codex_settings(&settings, CodexVersion::Reported(Some(SAFE_CODEX)))
        .await
        .unwrap();
}
