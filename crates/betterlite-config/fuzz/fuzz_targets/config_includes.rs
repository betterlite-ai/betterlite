#![no_main]

use std::collections::BTreeMap;
use std::fs;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::includes::load_file;
use libfuzzer_sys::fuzz_target;
use tempfile::tempdir;

fuzz_target!(|bytes: &[u8]| {
    let Ok(directory) = tempdir() else {
        return;
    };
    let root = directory.path().join("config.yaml");
    if fs::write(&root, bytes).is_err() {
        return;
    }
    for name in [
        "child.yaml",
        "included_models.yaml",
        "models_file_1.yaml",
        "models_file_2.yaml",
        "fallback_models.yaml",
        "conflict_models.yaml",
    ] {
        if fs::write(directory.path().join(name), bytes).is_err() {
            return;
        }
    }
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let _ = load_file(&loader, root);
});
