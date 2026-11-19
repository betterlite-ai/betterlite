use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use betterlite_pyhost::blocking_work::WorkTracker;
use betterlite_pyhost::host::{HostConfig, PythonHost};
use betterlite_pyhost::optional_library::{LibraryError, OptionalLibrary};
use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _, PyModule};
use pyo3::{Py, PyAny};

use super::super::programs::SESSION_LANG;
use super::super::sdk::{SdkSessions, open_failure};
use super::super::session::{RunResult, SessionError, SessionOpener, SessionOptions};

// the interpreter and sys.modules are shared, so the SDK tests run one at a time
static SDK_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const FAKE_SDK: &str = "
fail_run = False
_instances = []
def last_instance():
    return _instances[-1]
class _Result:
    def __init__(self, exit_code, stdout, stderr):
        self.exit_code = exit_code
        self.stdout = stdout
        self.stderr = stderr
class _Broken:
    pass
class _Inner:
    def __init__(self, manager):
        self.manager = manager
    def run(self, code):
        self.manager.events.append('run:' + code)
        if fail_run:
            raise RuntimeError('boom')
        if 'broken-result' in code:
            return _Broken()
        if 'no-stdout' in code:
            return _Result(3, None, 'bad')
        return _Result(0, 'out:' + code, None)
    def copy_to_runtime(self, source, target):
        self.manager.events.append('copy_to_runtime:' + source + '->' + target)
    def copy_from_runtime(self, source, target):
        self.manager.events.append('copy_from_runtime:' + source + '->' + target)
        with open(target, 'wb') as handle:
            handle.write(b'image-bytes')
class SandboxSession:
    def __init__(self, **kwargs):
        self.kwargs = kwargs
        self.events = []
        self.inner = _Inner(self)
        _instances.append(self)
    def __enter__(self):
        self.events.append('enter')
        return self.inner
    def __exit__(self, *args):
        self.events.append('exit')
";

fn start_host() -> PythonHost {
    PythonHost::start(&HostConfig::default()).unwrap()
}

fn install_fake(host: &PythonHost) {
    let source = CString::new(FAKE_SDK).expect("the fake source has no nul");
    host.attach(|py| {
        let module = PyModule::from_code(py, &source, c"llm_sandbox.py", c"llm_sandbox").unwrap();
        py.import("sys")
            .unwrap()
            .getattr("modules")
            .unwrap()
            .set_item("llm_sandbox", module)
            .unwrap();
    });
}

fn last_instance(host: &PythonHost) -> Py<PyAny> {
    host.attach(|py| {
        py.import("llm_sandbox")
            .unwrap()
            .getattr("last_instance")
            .unwrap()
            .call0()
            .unwrap()
            .unbind()
    })
}

fn events_of(host: &PythonHost, instance: &Py<PyAny>) -> Vec<String> {
    host.attach(|py| {
        instance
            .bind(py)
            .getattr("events")
            .unwrap()
            .extract::<Vec<String>>()
            .unwrap()
    })
}

fn exits_of(host: &PythonHost, instance: &Py<PyAny>) -> usize {
    events_of(host, instance)
        .iter()
        .filter(|event| event.as_str() == "exit")
        .count()
}

fn kwargs_of(host: &PythonHost, instance: &Py<PyAny>) -> BTreeMap<String, String> {
    host.attach(|py| {
        let kwargs = instance
            .bind(py)
            .getattr("kwargs")
            .unwrap()
            .cast_into::<PyDict>()
            .unwrap();
        let mut found = BTreeMap::new();
        for (key, value) in kwargs.iter() {
            found.insert(
                key.extract::<String>().unwrap(),
                value.repr().unwrap().extract::<String>().unwrap(),
            );
        }
        found
    })
}

fn sdk_sessions(host: &PythonHost) -> SdkSessions {
    SdkSessions {
        host: host.clone(),
        tracker: Arc::new(WorkTracker::new(2, Duration::from_secs(30))),
        library: OptionalLibrary::default(),
        deadline: Duration::from_secs(10),
    }
}

fn session_options(image: Option<&str>) -> SessionOptions {
    SessionOptions {
        lang: SESSION_LANG,
        verbose: false,
        image: image.map(str::to_owned),
    }
}

fn paired(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[tokio::test]
async fn sdk_session_passes_only_language_verbose_and_an_image() {
    let _guard = SDK_TESTS.lock().await;
    let host = start_host();
    install_fake(&host);
    let sessions = sdk_sessions(&host);
    let with_image = sessions
        .open(&session_options(Some("img:1")))
        .await
        .unwrap();
    assert_eq!(
        kwargs_of(&host, &last_instance(&host)),
        paired(&[
            ("image", "'img:1'"),
            ("lang", "'python'"),
            ("verbose", "False"),
        ])
    );
    drop(with_image);
    let without_image = sessions.open(&session_options(None)).await.unwrap();
    assert_eq!(
        kwargs_of(&host, &last_instance(&host)),
        paired(&[("lang", "'python'"), ("verbose", "False")])
    );
    drop(without_image);
}

#[tokio::test]
async fn sdk_run_reads_the_result_fields() {
    let _guard = SDK_TESTS.lock().await;
    let host = start_host();
    install_fake(&host);
    let session = sdk_sessions(&host)
        .open(&session_options(None))
        .await
        .unwrap();
    assert_eq!(
        session.run("print(1)").await.unwrap(),
        RunResult {
            exit_code: 0,
            stdout: Some("out:print(1)".to_owned()),
            stderr: None,
        }
    );
    assert_eq!(
        session.run("no-stdout").await.unwrap(),
        RunResult {
            exit_code: 3,
            stdout: None,
            stderr: Some("bad".to_owned()),
        }
    );
    assert_eq!(
        events_of(&host, &last_instance(&host)),
        vec![
            "enter".to_owned(),
            "run:print(1)".to_owned(),
            "run:no-stdout".to_owned(),
        ]
    );
}

#[tokio::test]
async fn sdk_copies_pass_both_paths() {
    let _guard = SDK_TESTS.lock().await;
    let host = start_host();
    install_fake(&host);
    let session = sdk_sessions(&host)
        .open(&session_options(None))
        .await
        .unwrap();
    let folder = tempfile::tempdir().unwrap();
    let local = folder.path().join("main.py");
    fs::write(&local, b"print(1)").unwrap();
    session
        .copy_to_runtime(&local, "/sandbox/main.py")
        .await
        .unwrap();
    let destination = folder.path().join("out.png");
    session
        .copy_from_runtime("/sandbox/out.png", &destination)
        .await
        .unwrap();
    assert_eq!(fs::read(&destination).unwrap(), b"image-bytes");
    assert_eq!(
        events_of(&host, &last_instance(&host)),
        vec![
            "enter".to_owned(),
            format!("copy_to_runtime:{}->/sandbox/main.py", local.display()),
            format!(
                "copy_from_runtime:/sandbox/out.png->{}",
                destination.display()
            ),
        ]
    );
}

#[tokio::test]
async fn sdk_close_exits_the_manager_once() {
    let _guard = SDK_TESTS.lock().await;
    let host = start_host();
    install_fake(&host);
    let session = sdk_sessions(&host)
        .open(&session_options(None))
        .await
        .unwrap();
    session.close().await.unwrap();
    assert_eq!(exits_of(&host, &last_instance(&host)), 1);
    session.close().await.unwrap();
    let instance = last_instance(&host);
    drop(session);
    assert_eq!(exits_of(&host, &instance), 1);
}

#[tokio::test]
async fn sdk_drop_exits_a_session_that_was_not_closed() {
    let _guard = SDK_TESTS.lock().await;
    let host = start_host();
    install_fake(&host);
    let instance = {
        let _session = sdk_sessions(&host)
            .open(&session_options(None))
            .await
            .unwrap();
        last_instance(&host)
    };
    assert_eq!(exits_of(&host, &instance), 1);
}

#[tokio::test]
async fn sdk_a_failed_call_names_the_operation() {
    let _guard = SDK_TESTS.lock().await;
    let host = start_host();
    install_fake(&host);
    host.attach(|py| {
        py.import("llm_sandbox")
            .unwrap()
            .setattr("fail_run", true)
            .unwrap();
    });
    let session = sdk_sessions(&host)
        .open(&session_options(None))
        .await
        .unwrap();
    let error = session.run("print(1)").await.unwrap_err();
    match error {
        SessionError::Call { operation, source } => {
            assert_eq!(operation, "run");
            assert!(source.message.contains("boom"));
        }
        other => panic!("expected a call error, got {other:?}"),
    }
}

#[tokio::test]
async fn sdk_a_result_without_fields_is_a_call_error() {
    let _guard = SDK_TESTS.lock().await;
    let host = start_host();
    install_fake(&host);
    let session = sdk_sessions(&host)
        .open(&session_options(None))
        .await
        .unwrap();
    let error = session.run("broken-result").await.unwrap_err();
    match error {
        SessionError::Call { operation, source } => {
            assert_eq!(operation, "run");
            assert!(source.message.contains("exit_code"));
        }
        other => panic!("expected a call error, got {other:?}"),
    }
}

#[test]
fn a_missing_library_maps_to_the_install_message() {
    let missing = LibraryError::NotInstalled {
        module: "llm_sandbox".to_owned(),
        install_hint: "llm-sandbox".to_owned(),
    };
    assert_eq!(open_failure(missing), SessionError::SdkMissing);
}

#[test]
fn another_library_error_starts_the_session_with_its_text() {
    let broken = LibraryError::NotCallable {
        module: "llm_sandbox".to_owned(),
        path: "SandboxSession".to_owned(),
    };
    let source = match open_failure(broken) {
        SessionError::Start { source } => source,
        other => panic!("expected a start error, got {other:?}"),
    };
    assert!(source.message.contains("llm_sandbox"));
}
