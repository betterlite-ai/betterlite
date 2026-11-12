#![cfg(test)]

#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt as _;
#[cfg(unix)]
use std::path::PathBuf;

use pyo3::types::PyAnyMethods as _;

use super::{DEFAULT_SHUTDOWN_GRACE, HostConfig, HostError, PythonHost, ensure_supported};

fn start_host() -> PythonHost {
    PythonHost::start(&HostConfig::default()).unwrap()
}

fn sys_path(host: &PythonHost) -> Vec<String> {
    host.attach(|py| {
        let sys = py.import("sys").unwrap();
        let path = sys.getattr("path").unwrap();
        path.extract::<Vec<String>>().unwrap()
    })
}

#[test]
fn start_puts_module_dirs_first_in_sys_path_in_order() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let config = HostConfig {
        module_dirs: vec![first.path().to_path_buf(), second.path().to_path_buf()],
        shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
    };
    let host = PythonHost::start(&config).unwrap();
    let path = sys_path(&host);
    assert_eq!(path.first().map(String::as_str), first.path().to_str());
    assert_eq!(path.get(1).map(String::as_str), second.path().to_str());
}

#[test]
fn a_second_start_keeps_each_module_dir_once() {
    let dir = tempfile::tempdir().unwrap();
    let wanted = dir.path().to_str().unwrap();
    let config = HostConfig {
        module_dirs: vec![dir.path().to_path_buf()],
        shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
    };
    let first = PythonHost::start(&config).unwrap();
    let second = PythonHost::start(&config).unwrap();
    let path = sys_path(&second);
    assert_eq!(path.first().map(String::as_str), Some(wanted));
    assert_eq!(
        path.iter().filter(|entry| entry.as_str() == wanted).count(),
        1
    );
    assert_eq!(first.version(), second.version());
}

#[test]
fn a_second_start_shares_the_interpreter() {
    let first = start_host();
    first.attach(|py| {
        let sys = py.import("sys").unwrap();
        sys.setattr("betterlite_b8040_marker", 4242_u32).unwrap();
    });
    let second = start_host();
    let marker = second.attach(|py| {
        let sys = py.import("sys").unwrap();
        sys.getattr("betterlite_b8040_marker")
            .unwrap()
            .extract::<u32>()
            .unwrap()
    });
    assert_eq!(marker, 4242);
}

#[test]
fn attach_runs_the_closure_with_the_interpreter() {
    let host = start_host();
    let major = host.attach(|py| py.version_info().major);
    assert_eq!(major, 3);
}

#[test]
fn version_returns_the_interpreter_triple() {
    let host = start_host();
    let reported = host.version();
    let live = host.attach(|py| {
        let info = py.version_info();
        (info.major, info.minor, info.patch)
    });
    assert_eq!(reported, live);
    assert!(
        reported >= (3, 10, 0),
        "abi3-py310 needs Python 3.10 or newer, found {reported:?}"
    );
}

#[test]
fn a_python_below_3_10_is_rejected() {
    assert!(matches!(
        ensure_supported((3, 9, 18)),
        Err(HostError::BadPythonVersion { found: (3, 9, 18) })
    ));
    assert!(matches!(ensure_supported((3, 10, 0)), Ok(())));
    assert!(matches!(ensure_supported((3, 13, 5)), Ok(())));
}

#[test]
fn import_check_accepts_an_installed_module() {
    let host = start_host();
    host.import_check("sys").unwrap();
}

#[test]
fn import_check_reports_the_import_error_text() {
    let host = start_host();
    match host
        .import_check("betterlite_b8040_missing_module")
        .unwrap_err()
    {
        HostError::Import { module, source } => {
            assert_eq!(module, "betterlite_b8040_missing_module");
            let text = source.to_string();
            assert!(
                text.contains("No module named 'betterlite_b8040_missing_module'"),
                "got: {text}"
            );
        }
        other @ (HostError::BadPythonVersion { .. }
        | HostError::ModuleDirNotUtf8 { .. }
        | HostError::Python { .. }) => panic!("expected an import error, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn a_module_dir_that_is_not_utf8_is_refused() {
    let path = PathBuf::from(OsString::from_vec(vec![b'/', b'm', 0xff]));
    let config = HostConfig {
        module_dirs: vec![path.clone()],
        shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
    };
    match PythonHost::start(&config).unwrap_err() {
        HostError::ModuleDirNotUtf8 { path: reported } => assert_eq!(reported, path),
        other @ (HostError::BadPythonVersion { .. }
        | HostError::Import { .. }
        | HostError::Python { .. }) => {
            panic!("expected a non-UTF-8 module dir error, got {other:?}")
        }
    }
}
