#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use secrecy::SecretString;
use std::process;

use super::{LocalSandbox, filtered_environ, is_secret_env_name};
use crate::sandbox::{Sandbox, SandboxCommand};

fn tempdir() -> PathBuf {
    let path = std::env::temp_dir().join(format!("betterlite-local-test-{}", process::id()));
    drop(fs::remove_dir_all(&path));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn filters_provider_credentials_but_extra_values_win() {
    assert!(is_secret_env_name("openai_api_key"));
    assert!(is_secret_env_name("SERVICE_SECRET_VALUE"));
    assert!(!is_secret_env_name("PATH"));
    let mut extra = BTreeMap::new();
    extra.insert("OPENAI_API_KEY".to_owned(), SecretString::from("explicit"));
    assert_eq!(
        filtered_environ(&extra)
            .get("OPENAI_API_KEY")
            .map(String::as_str),
        Some("explicit")
    );
}

#[tokio::test]
async fn executes_commands_and_confines_paths() {
    let root = tempdir();
    let sandbox = LocalSandbox::new(&root).unwrap();
    let command = SandboxCommand {
        argv: vec!["printf".to_owned(), "hello".to_owned()],
        ..SandboxCommand::default()
    };
    let run = sandbox
        .run(&command, Duration::from_secs(2), 1024)
        .await
        .unwrap();
    assert_eq!(run.stdout, "hello");
    assert!(sandbox.resolve_path("../outside").is_err());
}

#[tokio::test]
async fn reads_writes_and_cleans_tempdirs() {
    let root = tempdir();
    let sandbox = LocalSandbox::new(&root).unwrap();
    sandbox.write("nested/file", b"hello\xff").await.unwrap();
    assert_eq!(
        sandbox.read("nested/file", 100).await.unwrap(),
        b"hello\xff"
    );
    let temporary = sandbox.tempdir().await.unwrap();
    let temporary_path = PathBuf::from(&temporary);
    assert!(temporary.starts_with("/"));
    assert!(temporary_path.is_dir());
    sandbox.close().await.unwrap();
    assert!(!temporary_path.exists());
}

#[tokio::test]
async fn rejects_empty_commands_and_file_cwds() {
    let root = tempdir();
    let sandbox = LocalSandbox::new(&root).unwrap();
    let empty = SandboxCommand::default();
    assert!(sandbox.exec(&empty).await.is_err());
    fs::write(root.join("file"), b"x").unwrap();
    let command = SandboxCommand {
        argv: vec!["true".to_owned()],
        cwd: Some("file".to_owned()),
        ..SandboxCommand::default()
    };
    let error = match sandbox.exec(&command).await {
        Ok(_) => panic!("a file is not a valid cwd"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("cwd is not a directory"));
}
