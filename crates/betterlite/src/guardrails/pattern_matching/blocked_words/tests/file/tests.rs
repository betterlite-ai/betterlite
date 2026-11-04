#![cfg(test)]

use super::{ErrorKind, WordsError, fs, load};
use betterlite_config::sections::content_filter_records::ContentFilterAction;
use rstest::rstest;

#[rstest]
#[case::missing_file(false)]
#[case::valid_file(true)]
fn pattern_matching_behavior(#[case] exists: bool) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("words.yaml");
    if exists {
        fs::write(
            &path,
            "blocked_words: [{keyword: SECRET, action: MASK, description: hidden}]",
        )
        .unwrap();
        let words = load(&path).unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].keyword, "secret");
        assert_eq!(words[0].action, ContentFilterAction::Mask);
        assert_eq!(words[0].description.as_deref(), Some("hidden"));
    } else {
        assert!(
            matches!(load(&path).unwrap_err(), WordsError::Read { source, .. } if source.kind() == ErrorKind::NotFound)
        );
    }
}
