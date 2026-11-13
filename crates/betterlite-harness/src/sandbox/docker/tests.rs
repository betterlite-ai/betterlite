#![cfg(test)]

mod files;
mod runtime;
mod sha256sum;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use secrecy::SecretString;
use tempfile::{Builder, TempDir};

use super::DockerSandbox;
use super::args::normalize_path;
use crate::sandbox::Sandbox;
use crate::sandbox::SandboxCommand;

fn tempdir() -> TempDir {
    Builder::new()
        .prefix("betterlite-docker-test-")
        .tempdir()
        .unwrap()
}

fn fake_docker(dir: &TempDir, body: &str) -> PathBuf {
    let script = format!(
        "#!/bin/sh\ndir='{}'\nlog=\"$dir/argv\"\n{{ for arg do printf '%s\\n' \"$arg\"; done; printf '===\\n'; }} >> \"$log\"\n{body}\n",
        dir.path().display()
    );
    let path = dir.path().join("docker");
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn docker_script_with(dir: &TempDir, run_body: &str, exec_body: &str, rm_body: &str) -> PathBuf {
    let body = format!(
        "if [ \"$1\" = run ]; then\n{run_body}\nfi\nif [ \"$1\" = exec ]; then\n{exec_body}\nfi\nif [ \"$1\" = rm ]; then\n{rm_body}\nfi\nexit 0"
    );
    fake_docker(dir, &body)
}

fn docker_script(dir: &TempDir, exec_body: &str) -> PathBuf {
    docker_script_with(dir, "printf '  container-1  \\n'", exec_body, "exit 0")
}

fn invocations(dir: &TempDir) -> Vec<Vec<String>> {
    let Ok(text) = fs::read_to_string(dir.path().join("argv")) else {
        return Vec::new();
    };
    text.split("===\n")
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.lines().map(str::to_owned).collect())
        .collect()
}

fn invocations_of(calls: &[Vec<String>], verb: &str) -> Vec<Vec<String>> {
    calls
        .iter()
        .filter(|args| args.first().map(String::as_str) == Some(verb))
        .cloned()
        .collect()
}

fn sandbox(program: PathBuf) -> DockerSandbox {
    DockerSandbox::new("test-image".to_owned(), Vec::new(), None, Vec::new(), None)
        .unwrap()
        .with_docker_program(program)
}

fn command(argv: &[&str]) -> SandboxCommand {
    SandboxCommand {
        argv: argv.iter().map(|part| (*part).to_owned()).collect(),
        env: BTreeMap::new(),
        cwd: None,
    }
}

#[test]
fn rejects_an_empty_image() {
    let Err(error) = DockerSandbox::new(String::new(), Vec::new(), None, Vec::new(), None) else {
        panic!("an empty image must be rejected");
    };
    assert_eq!(
        error.to_string(),
        "the sandbox failed: docker sandbox needs an image"
    );
}

#[test]
fn rejects_a_relative_workdir() {
    let Err(error) = DockerSandbox::new(
        "img".to_owned(),
        Vec::new(),
        Some("work".to_owned()),
        Vec::new(),
        None,
    ) else {
        panic!("a relative workdir must be rejected");
    };
    assert_eq!(
        error.to_string(),
        "the sandbox failed: docker workdir must be absolute: work"
    );
}

#[test]
fn rejects_an_empty_workdir() {
    let Err(error) = DockerSandbox::new(
        "img".to_owned(),
        Vec::new(),
        Some(String::new()),
        Vec::new(),
        None,
    ) else {
        panic!("an empty workdir is not absolute");
    };
    assert_eq!(
        error.to_string(),
        "the sandbox failed: docker workdir must be absolute: "
    );
}

#[test]
fn defaults_to_the_workspace_directory() {
    let sandbox = DockerSandbox::new("img".to_owned(), Vec::new(), None, Vec::new(), None).unwrap();
    assert_eq!(Sandbox::workdir(&sandbox), "/workspace");
    assert_eq!(
        sandbox.run_args(),
        vec![
            "run",
            "-d",
            "--rm",
            "--add-host=host.docker.internal:host-gateway",
            "-w",
            "/workspace",
            "img",
            "sleep",
            "infinity",
        ]
    );
}

#[test]
fn normalizes_the_workdir_like_posixpath() {
    let sandbox = DockerSandbox::new(
        "img".to_owned(),
        Vec::new(),
        Some("//workspace/./sub/../root/".to_owned()),
        Vec::new(),
        None,
    )
    .unwrap();
    assert_eq!(Sandbox::workdir(&sandbox), "//workspace/root");
}

#[test]
fn normalizes_mount_host_paths_and_keeps_the_last_container_path() {
    let mounts = vec![
        ("/host/one".to_owned(), "/first".to_owned()),
        ("/host/two/./".to_owned(), "/second".to_owned()),
        ("/host/one/../one".to_owned(), "/third".to_owned()),
    ];
    let sandbox = DockerSandbox::new("img".to_owned(), mounts, None, Vec::new(), None).unwrap();
    assert_eq!(
        sandbox.run_args(),
        vec![
            "run",
            "-d",
            "--rm",
            "--add-host=host.docker.internal:host-gateway",
            "-v",
            "/host/one:/third",
            "-v",
            "/host/two:/second",
            "-w",
            "/workspace",
            "img",
            "sleep",
            "infinity",
        ]
    );
}

#[test]
fn absolutizes_relative_mount_hosts_without_resolving_symlinks() {
    let mounts = vec![("nested/../file".to_owned(), "/target".to_owned())];
    let sandbox = DockerSandbox::new("img".to_owned(), mounts, None, Vec::new(), None).unwrap();
    let current = env::current_dir().unwrap();
    let joined = current.join("file");
    let expected = joined.to_str().unwrap().to_owned();
    let args = sandbox.run_args();
    assert_eq!(args.get(5), Some(&format!("{expected}:/target")));
}

#[test]
fn run_args_carry_the_name_mounts_and_env_in_order() {
    let env = vec![
        ("ZED".to_owned(), SecretString::from("two")),
        ("ALPHA".to_owned(), SecretString::from("one")),
    ];
    let mounts = vec![("/host".to_owned(), "/container".to_owned())];
    let sandbox = DockerSandbox::new(
        "img".to_owned(),
        mounts,
        Some("/work".to_owned()),
        env,
        Some("box".to_owned()),
    )
    .unwrap();
    assert_eq!(
        sandbox.run_args(),
        vec![
            "run",
            "-d",
            "--rm",
            "--add-host=host.docker.internal:host-gateway",
            "--name",
            "box",
            "-v",
            "/host:/container",
            "-e",
            "ZED=two",
            "-e",
            "ALPHA=one",
            "-w",
            "/work",
            "img",
            "sleep",
            "infinity",
        ]
    );
}

#[test]
fn exec_args_place_the_workdir_env_and_container_command() {
    let mut env = BTreeMap::new();
    env.insert("B".to_owned(), SecretString::from("2"));
    env.insert("A".to_owned(), SecretString::from("1"));
    let sandbox = DockerSandbox::new("img".to_owned(), Vec::new(), None, Vec::new(), None).unwrap();
    assert_eq!(
        sandbox.exec_args(
            "cid",
            &["ls".to_owned(), "-l".to_owned()],
            &env,
            Some("sub"),
        ),
        vec![
            "exec",
            "-i",
            "-w",
            "/workspace/sub",
            "-e",
            "A=1",
            "-e",
            "B=2",
            "cid",
            "ls",
            "-l"
        ]
    );
}

#[rstest::rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn exec_args_use_the_workdir_when_cwd_is_absent_or_empty(#[case] cwd: Option<&str>) {
    let sandbox = DockerSandbox::new("img".to_owned(), Vec::new(), None, Vec::new(), None).unwrap();
    let args = sandbox.exec_args("cid", &["true".to_owned()], &BTreeMap::new(), cwd);
    assert_eq!(args.get(3).map(String::as_str), Some("/workspace"));
}

#[test]
fn exec_args_keep_an_absolute_cwd() {
    let sandbox = DockerSandbox::new("img".to_owned(), Vec::new(), None, Vec::new(), None).unwrap();
    let args = sandbox.exec_args(
        "cid",
        &["true".to_owned()],
        &BTreeMap::new(),
        Some("/elsewhere"),
    );
    assert_eq!(args.get(3).map(String::as_str), Some("/elsewhere"));
}

#[rstest::rstest]
#[case::empty("", "/workspace")]
#[case::dot(".", "/workspace")]
#[case::dot_dot("a/../../b", "/b")]
#[case::leading_double_slash("//host/share", "//host/share")]
#[case::leading_triple_slash("///host//share", "/host/share")]
#[case::absolute_parent("/../etc", "/etc")]
#[case::root("/", "/")]
#[case::double_slash_root("//", "//")]
fn container_path_normalizes_like_posixpath(#[case] path: &str, #[case] expected: &str) {
    let sandbox = DockerSandbox::new("img".to_owned(), Vec::new(), None, Vec::new(), None).unwrap();
    assert_eq!(sandbox.container_path(path), expected);
}

#[test]
fn container_path_joins_relative_paths_onto_the_workdir() {
    let sandbox = DockerSandbox::new(
        "img".to_owned(),
        Vec::new(),
        Some("//w".to_owned()),
        Vec::new(),
        None,
    )
    .unwrap();
    assert_eq!(sandbox.container_path("a/../b"), "//w/b");
    assert_eq!(sandbox.container_path("//x"), "//x");
}

#[rstest::rstest]
#[case::empty("", ".")]
#[case::dot(".", ".")]
#[case::dot_dot("..", "..")]
#[case::parent_stays("../b", "../b")]
#[case::relative_parent("a/../../b", "../b")]
#[case::deep_parent("a/b/../../../c", "../c")]
#[case::absolute_parent_at_root("/a/../..", "/")]
#[case::double_slash_kept("//a/../b", "//b")]
#[case::four_slashes("////a", "/a")]
#[case::double_slash_root("//", "//")]
fn normalize_path_matches_posixpath(#[case] path: &str, #[case] expected: &str) {
    assert_eq!(normalize_path(path), expected);
}

#[test]
fn reports_a_container_with_the_docker_host_alias() {
    let sandbox = DockerSandbox::new("img".to_owned(), Vec::new(), None, Vec::new(), None).unwrap();
    assert!(Sandbox::is_container(&sandbox));
    assert_eq!(
        Sandbox::host_url(&sandbox, 8123),
        "http://host.docker.internal:8123"
    );
}

#[test]
fn missing_docker_program_gives_the_install_hint() {
    let error = super::error::DockerSandboxError::DockerMissing;
    assert_eq!(
        error.to_string(),
        "docker sandbox requires the `docker` CLI on PATH; install Docker or use sandbox.local(path)"
    );
}
