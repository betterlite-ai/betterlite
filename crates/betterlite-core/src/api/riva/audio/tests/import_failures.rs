use betterlite_pyhost::host::PythonHost;
use pyo3::types::PyAnyMethods as _;

use super::{
    PROBE, PYTHON, RivaAudioError, decode, host_with_numpy, install, modules, reset_fakes,
    soundfile_failure, soundfile_source,
};

const FINDER: &str = "import sys
class _BrokenFinder:
    def find_spec(self, name, path=None, target=None):
        if name == '__BLOCKED__':
            raise RuntimeError('__BLOCKED__ import exploded')
        return None
sys.meta_path.insert(0, _BrokenFinder())
";

fn install_broken_import(host: &PythonHost, blocked: &str, soundfile: &str) {
    let source = FINDER.replace("__BLOCKED__", blocked);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(py, "soundfile", soundfile);
        install(py, "broken_finder", &source);
        drop(modules(py).del_item(blocked));
    });
}

// the finder sits first on meta_path, and the interpreter outlives the test
struct RemovedFinder<'host> {
    host: &'host PythonHost,
}

impl Drop for RemovedFinder<'_> {
    fn drop(&mut self) {
        self.host.attach(|py| {
            let meta_path = py.import("sys").unwrap().getattr("meta_path").unwrap();
            meta_path.call_method1("pop", (0_i64,)).unwrap();
            drop(modules(py).del_item("broken_finder"));
        });
    }
}

#[test]
fn reports_a_broken_audioread_import_as_a_python_failure() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    install_broken_import(
        &host,
        "audioread",
        &soundfile_failure("cannot open sound file"),
    );
    let _finder = RemovedFinder { host: &host };
    let error = decode(&host, b"staged bytes").unwrap_err();
    assert!(matches!(
        error,
        RivaAudioError::Python {
            operation: "import audioread",
            ..
        }
    ));
    assert_eq!(
        error.to_string(),
        "import audioread failed: RuntimeError: audioread import exploded"
    );
    assert_eq!(error.response().status_code, 500);
}

#[test]
fn reports_a_broken_numpy_import_as_a_python_failure() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    install_broken_import(&host, "numpy", &soundfile_failure("cannot open sound file"));
    let _finder = RemovedFinder { host: &host };
    let error = decode(&host, b"staged bytes").unwrap_err();
    assert!(matches!(
        error,
        RivaAudioError::Python {
            operation: "import numpy",
            ..
        }
    ));
    assert_eq!(
        error.to_string(),
        "import numpy failed: RuntimeError: numpy import exploded"
    );
    assert_eq!(error.response().status_code, 500);
}

#[test]
fn reports_a_broken_soxr_import_as_a_python_failure() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    install_broken_import(&host, "soxr", &soundfile_source("[0.0, 0.25]", 2, 1, 8_000));
    let _finder = RemovedFinder { host: &host };
    let error = decode(&host, b"fake audio").unwrap_err();
    assert!(matches!(
        error,
        RivaAudioError::Python {
            operation: "soxr.resample",
            ..
        }
    ));
    assert_eq!(
        error.to_string(),
        "soxr.resample failed: RuntimeError: soxr import exploded"
    );
    assert_eq!(error.response().status_code, 500);
}
