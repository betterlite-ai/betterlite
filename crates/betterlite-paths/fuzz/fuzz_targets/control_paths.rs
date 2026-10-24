#![no_main]

use betterlite_paths::paths::delete::remove;
use betterlite_paths::paths::lookup::find;
use betterlite_paths::paths::segments::parse;
use betterlite_paths::paths::{Document, PathLimits};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(path) = std::str::from_utf8(bytes) else {
        return;
    };
    let limits = PathLimits {
        max_path_bytes: 64,
        max_depth: 4,
        max_selectors: 2,
        max_visits: 16,
    };
    let document: Document = serde_json::from_str(r#"{"a": [{"b": 1}], "c": {"d": 2}}"#).unwrap();
    let _segments = parse(&limits, path);
    let _found = find(&limits, &document, path);
    let _deleted = remove(&limits, &document, path);
});
