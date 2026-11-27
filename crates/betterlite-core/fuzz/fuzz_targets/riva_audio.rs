#![no_main]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use betterlite_core::api::riva::audio::decode_riva_audio_blocking;
use betterlite_pyhost::host::{HostConfig, PythonHost};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let config = HostConfig {
        module_dirs: oracle_module_dirs(),
        shutdown_grace: Duration::from_secs(1),
    };
    let Ok(host) = PythonHost::start(&config) else {
        return;
    };
    drop(decode_riva_audio_blocking(&host, data));
});

// the oracle environment carries soundfile, audioread and numpy when uv has synced it
fn oracle_module_dirs() -> Vec<PathBuf> {
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tools/oracle/litellm/.venv/lib");
    let Ok(entries) = fs::read_dir(lib) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path().join("site-packages"))
        .filter(|path| path.is_dir())
        .collect()
}
