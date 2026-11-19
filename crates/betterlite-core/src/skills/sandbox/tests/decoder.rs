use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::super::collector::{MAX_COLLECTED_BYTES, content_of};
use super::super::decoder::{
    ListingRefusal, MAX_LISTING_BYTES, MAX_LISTING_ENTRIES, decode_generated_paths,
};
use super::super::programs::{
    LISTING_CODE, REQUIREMENTS_FILE, mime_type, pip_install_code, wrapped_code,
};

#[rstest]
#[case::gif("chart.gif", "image/gif")]
#[case::png("chart.png", "image/png")]
#[case::jpg("chart.jpg", "image/jpeg")]
#[case::jpeg("chart.jpeg", "image/jpeg")]
#[case::pdf("chart.pdf", "application/pdf")]
#[case::csv("chart.csv", "text/csv")]
#[case::json("chart.json", "application/json")]
#[case::txt("chart.txt", "text/plain")]
#[case::other("chart.webp", "application/octet-stream")]
fn the_mime_type_follows_the_extension(#[case] name: &str, #[case] expected: &str) {
    assert_eq!(mime_type(name), expected);
    assert_eq!(mime_type(&format!("/sandbox/nested/{name}")), expected);
}

#[rstest]
#[case::upper("chart.PNG", "image/png")]
#[case::jpeg_upper("chart.JPEG", "image/jpeg")]
fn the_mime_type_ignores_the_extension_case(#[case] name: &str, #[case] expected: &str) {
    assert_eq!(mime_type(name), expected);
}

#[test]
fn the_code_wrapper_changes_directory_and_prepends_the_path() {
    assert_eq!(
        wrapped_code("x = 1"),
        "\nimport os\nos.chdir('/sandbox')\nimport sys\nsys.path.insert(0, '/sandbox')\n\nx = 1\n"
    );
}

#[test]
fn the_install_code_runs_pip_with_the_requirements_filename() {
    assert_eq!(
        pip_install_code(REQUIREMENTS_FILE),
        "\nimport subprocess\nimport sys\nsubprocess.run(\n    [sys.executable, '-m', 'pip', 'install', '-r', '.litellm_requirements.txt'],\n    check=True,\n    cwd='/sandbox',\n)\n"
    );
}

#[test]
fn the_listing_walk_names_every_collected_extension() {
    assert_eq!(
        LISTING_CODE,
        "\nimport os\nimport json\nfiles = []\nfor root, dirs, filenames in os.walk('/sandbox'):\n    for f in filenames:\n        if f.endswith(('.gif', '.png', '.jpg', '.jpeg', '.pdf', '.csv', '.json')):\n            files.append(os.path.join(root, f))\nprint(json.dumps(files))\n"
    );
}

#[test]
fn the_caps_hold_their_documented_values() {
    assert_eq!(MAX_LISTING_BYTES, 1_048_576);
    assert_eq!(MAX_LISTING_ENTRIES, 4096);
    assert_eq!(MAX_COLLECTED_BYTES, 67_108_864);
}

#[rstest]
#[case::two_paths("[\"/sandbox/a.png\", \"/sandbox/b.pdf\"]", 2)]
#[case::empty("[]", 0)]
fn a_valid_listing_keeps_every_path(#[case] text: &str, #[case] expected: usize) {
    let decoded = decode_generated_paths(text);
    assert_eq!(decoded.paths.len(), expected);
    assert_eq!(decoded.refusal, None);
}

#[rstest]
#[case::not_json("not json")]
#[case::an_object("{\"path\": \"/sandbox/a.png\"}")]
#[case::a_number_list("[1, 2]")]
#[case::a_string("\"/sandbox/a.png\"")]
fn a_malformed_listing_becomes_empty(#[case] text: &str) {
    let decoded = decode_generated_paths(text);
    assert!(decoded.paths.is_empty());
    assert_eq!(decoded.refusal, Some(ListingRefusal::Malformed));
}

#[test]
fn a_listing_at_the_byte_limit_is_still_read() {
    let text = "x".repeat(MAX_LISTING_BYTES);
    let decoded = decode_generated_paths(&text);
    assert_eq!(decoded.refusal, Some(ListingRefusal::Malformed));
}

#[test]
fn a_listing_past_the_byte_limit_is_refused() {
    let text = "x".repeat(MAX_LISTING_BYTES + 1);
    let decoded = decode_generated_paths(&text);
    assert!(decoded.paths.is_empty());
    assert_eq!(decoded.refusal, Some(ListingRefusal::TooLarge));
}

#[test]
fn a_listing_at_the_entry_limit_keeps_its_entries() {
    let paths = vec!["/sandbox/a.png"; MAX_LISTING_ENTRIES];
    let text = serde_json::to_string(&paths).unwrap();
    let decoded = decode_generated_paths(&text);
    assert_eq!(decoded.paths.len(), MAX_LISTING_ENTRIES);
    assert_eq!(decoded.refusal, None);
}

#[test]
fn a_listing_past_the_entry_limit_is_truncated() {
    let paths = vec!["/sandbox/a.png"; MAX_LISTING_ENTRIES + 1];
    let text = serde_json::to_string(&paths).unwrap();
    let decoded = decode_generated_paths(&text);
    assert_eq!(decoded.paths.len(), MAX_LISTING_ENTRIES);
    assert_eq!(decoded.refusal, Some(ListingRefusal::TooManyEntries));
}

#[tokio::test]
async fn the_collected_bytes_stop_at_the_cap() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("a.png");
    fs::write(&path, b"a").unwrap();
    assert_eq!(
        content_of(&path, MAX_COLLECTED_BYTES - 1).await.unwrap(),
        Some(b"a".to_vec())
    );
    assert_eq!(content_of(&path, MAX_COLLECTED_BYTES).await.unwrap(), None);
    let empty = folder.path().join("empty.png");
    fs::write(&empty, b"").unwrap();
    assert_eq!(
        content_of(&empty, MAX_COLLECTED_BYTES).await.unwrap(),
        Some(Vec::new())
    );
    assert_eq!(
        content_of(&folder.path().join("missing.png"), 0)
            .await
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound
    );
}

#[rstest]
fn replays_generated_paths(#[files("fuzz/corpus/generated_paths/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = String::from_utf8(bytes) else {
        return;
    };
    let decoded = decode_generated_paths(&text);
    assert!(decoded.paths.len() <= MAX_LISTING_ENTRIES);
}
