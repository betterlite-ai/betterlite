#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::Duration;

use serde_json::{Value, json};

use super::answers::{Answers, Layer, key};
use crate::case::helper::HelperCase;
use crate::error::HelperRunError;

#[rstest::rstest]
#[case::hung_child("exec sleep 5", "helper timed out and was reaped")]
#[case::excess_output("head -c 1048577 /dev/zero", "helper output exceeded 1 MiB")]
#[case::failed_child("echo e >&2; exit 3", "helper exited with exit status: 3: e\n")]
fn helper_runner_tests_behavior(#[case] script: &str, #[case] expected: &str) {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    let timeout = Duration::from_millis(300);
    let error = super::python::run_bounded(&mut command, Vec::new(), timeout).unwrap_err();
    assert_eq!(error.to_string(), expected);
    let unreaped = fs::read_to_string("/proc/thread-self/children").unwrap();
    assert_eq!(unreaped, "");
}

#[test]
fn helper_case_cannot_disable_the_local_cost_map() {
    let case: HelperCase = serde_saphyr::from_str(
        "kind: helper\nname: reserved\ncall: {url_path: {function: segment, value: ., field_name: id}}\nenv: {LITELLM_LOCAL_MODEL_COST_MAP: 'False'}\ncompare: {ignore: []}\n",
    )
    .expect("helper case should parse");
    let mut command = Command::new("true");
    super::python::configure_environment(&mut command, &BTreeMap::new(), &case.env);
    let cost_map = command
        .get_envs()
        .find(|(name, _)| *name == "LITELLM_LOCAL_MODEL_COST_MAP");
    assert_eq!(
        cost_map.and_then(|(_, value)| value),
        Some(OsStr::new("True"))
    );
}

#[rstest::rstest]
#[case::nested_number("/value/interval/ms", "5")]
#[case::object("/value/interval", "{\"ms\":5}")]
#[case::missing("/value/missing", "absent")]
fn a_failed_helper_case_shows_the_value_at_the_pointer(#[case] pointer: &str, #[case] shown: &str) {
    let output = super::HelperOutput {
        ok: true,
        value: Some(serde_json::from_str(r#"{"interval":{"ms":5}}"#).unwrap()),
        error: None,
        warnings: Vec::new(),
        effects: Vec::new(),
    };
    assert_eq!(super::value_at(&output, pointer), shown);
}

#[test]
fn a_failed_helper_case_shows_at_most_400_characters() {
    let output = super::HelperOutput {
        ok: true,
        value: Some(Value::String("x".repeat(1000))),
        error: None,
        warnings: Vec::new(),
        effects: Vec::new(),
    };
    assert_eq!(super::value_at(&output, "/value").chars().count(), 400);
}

#[rstest::rstest]
#[case::at_limit(0x0010_0000, false)]
#[case::over_limit(0x0010_0001, true)]
fn helper_output_limit_is_enforced(#[case] size: usize, #[case] excess: bool) {
    let bytes = vec![b'x'; size];
    let (output, observed_excess) = super::python::read_limited(Cursor::new(bytes)).unwrap();
    assert_eq!(output.len(), 0x0010_0000.min(size));
    assert_eq!(observed_excess, excess);
}

fn answer_scratch(name: &str) -> PathBuf {
    let scratch = env::temp_dir().join(format!("betterlite-answers-{}-{name}", process::id()));
    for keyed in [
        "tools/oracle/litellm/uv.lock",
        "tools/oracle/litellm/pyproject.toml",
        "tools/oracle/litellm/.python-version",
        "tools/oracle/helpers/dispatch.py",
    ] {
        let path = scratch.join(keyed);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, keyed).unwrap();
    }
    scratch
}

fn url_path_call() -> super::HelperCall {
    serde_saphyr::from_str("url_path: {function: segment, value: a/b, field_name: id}").unwrap()
}

#[test]
fn answer_cache_reads_the_shared_layer_first() {
    let root = answer_scratch("layers");
    let call = url_path_call();
    let case_env = BTreeMap::new();
    let name = format!("{}.json", key(&root, &call, &case_env).unwrap());
    let answers = Answers {
        shared: root.join("shared"),
        own: root.join("own"),
        write: Layer::Own,
        process_env: super::python::process_environment(),
    };
    fs::create_dir_all(&answers.shared).unwrap();
    fs::create_dir_all(&answers.own).unwrap();
    fs::write(
        answers.shared.join(&name),
        r#"{"ok":true,"value":"shared"}"#,
    )
    .unwrap();
    fs::write(answers.own.join(&name), r#"{"ok":true,"value":"own"}"#).unwrap();
    let shared = answers.answer(&root, &call, &case_env).unwrap();
    fs::remove_file(answers.shared.join(&name)).unwrap();
    let own = answers.answer(&root, &call, &case_env).unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(shared.value, Some(json!("shared")));
    assert_eq!(own.value, Some(json!("own")));
}

#[test]
fn answer_cache_key_follows_the_helper_module_the_pin_and_the_env() {
    let root = answer_scratch("key");
    let call = url_path_call();
    let case_env = BTreeMap::new();
    let mut keys = vec![key(&root, &call, &case_env).unwrap()];
    let module = root.join("tools/oracle/helpers/url_path.py");
    fs::write(&module, "first").unwrap();
    keys.push(key(&root, &call, &case_env).unwrap());
    fs::write(&module, "second").unwrap();
    keys.push(key(&root, &call, &case_env).unwrap());
    fs::write(root.join("tools/oracle/litellm/uv.lock"), "bumped").unwrap();
    keys.push(key(&root, &call, &case_env).unwrap());
    let named = BTreeMap::from([("A".to_owned(), "1".to_owned())]);
    keys.push(key(&root, &call, &named).unwrap());
    fs::remove_file(root.join("tools/oracle/helpers/dispatch.py")).unwrap();
    let missing = key(&root, &call, &case_env).unwrap_err();
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(keys.iter().collect::<BTreeSet<_>>().len(), 5);
    assert!(matches!(missing, HelperRunError::Filesystem { .. }));
}

#[test]
fn python_failures_carry_the_exception_status() {
    let root = Path::new(super::ORACLE_DIR).join("../..");
    super::sync_litellm(&root).unwrap();
    let case_env = BTreeMap::new();
    let process_env = super::python::process_environment();
    let role_call: super::HelperCall = serde_saphyr::from_str(
        "builtin_prompts: {family: ollama, messages: [{role: other, content: x}], model: injected-model}",
    )
    .unwrap();
    let role_error = super::python::python_stdout(&root, &role_call, &case_env, &process_env)
        .and_then(|stdout_output| {
            serde_json::from_slice::<super::HelperOutput>(&stdout_output)
                .map_err(HelperRunError::HelperJson)
        })
        .unwrap()
        .error
        .unwrap();
    assert_eq!(role_error.status, Some(400));

    let path_call: super::HelperCall =
        serde_saphyr::from_str("url_path: {function: segment, value: '..', field_name: file_id}")
            .unwrap();
    let path_error = super::python::python_stdout(&root, &path_call, &case_env, &process_env)
        .and_then(|stdout_output| {
            serde_json::from_slice::<super::HelperOutput>(&stdout_output)
                .map_err(HelperRunError::HelperJson)
        })
        .unwrap()
        .error
        .unwrap();
    assert_eq!(path_error.status, None);
}

#[test]
fn answer_cache_stores_a_miss_in_the_write_layer_only() {
    let root = Path::new(super::ORACLE_DIR).join("../..");
    let scratch = answer_scratch("miss");
    let answers = Answers {
        shared: scratch.join("shared"),
        own: scratch.join("own"),
        write: Layer::Shared,
        process_env: super::python::process_environment(),
    };
    let call = url_path_call();
    let case_env = BTreeMap::new();
    let output = answers.answer(&root, &call, &case_env).unwrap();
    let stored: Vec<OsString> = fs::read_dir(&answers.shared)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    let name = format!("{}.json", key(&root, &call, &case_env).unwrap());
    let kept: super::HelperOutput =
        serde_json::from_slice(&fs::read(answers.shared.join(&name)).unwrap()).unwrap();
    let own_written = answers.own.exists();
    fs::remove_dir_all(&scratch).unwrap();
    assert_eq!(stored, [OsString::from(name)]);
    assert!(!own_written);
    assert!(output.ok);
    assert_eq!(output, kept);
}

#[test]
fn litellm_sync_reports_a_missing_project() {
    let error = super::sync_litellm(Path::new("/definitely-missing-betterlite-oracle-project"))
        .unwrap_err();
    assert!(matches!(error, HelperRunError::ProcessSync(_)));
}
