use std::fs;
use std::path::{Path, PathBuf};
use std::str;

use crate::database_refresh_loop::extraction::extract_refresh_password;

fn replay(path: &Path) -> Option<String> {
    let name = path.file_name().and_then(|name| name.to_str())?.to_owned();
    let bytes = fs::read(path).ok()?;
    let Ok(text) = str::from_utf8(&bytes) else {
        return None;
    };
    let extracted = extract_refresh_password(text).is_some();
    let expected = if name.ends_with("-extracted") {
        true
    } else if name.ends_with("-none") {
        false
    } else {
        return None;
    };
    (extracted != expected).then(|| format!("{name}: extracted {extracted}"))
}

#[test]
fn replays_the_refresh_password_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/refresh_password");
    let paths: Vec<PathBuf> = fs::read_dir(corpus)
        .expect("the refresh password corpus exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .collect();
    assert!(!paths.is_empty());
    let mismatches: Vec<String> = paths.iter().filter_map(|path| replay(path)).collect();
    assert_eq!(mismatches, Vec::<String>::new());
}
