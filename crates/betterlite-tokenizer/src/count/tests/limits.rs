use std::num::NonZeroUsize;

use crate::count::settings::Settings;

#[test]
fn encode_limits_follow_the_exact_threshold() {
    let settings = Settings {
        disabled: false,
        max_exact_chars: NonZeroUsize::new(64).unwrap(),
        chunk_chars: NonZeroUsize::new(1024).unwrap(),
    };
    let limits = settings.encode_limits().unwrap();
    assert_eq!(limits.input_bytes, 256);
    assert_eq!(limits.output_ids, 256);
}

#[test]
fn default_encode_limits_match_the_default_threshold() {
    let limits = Settings::default().encode_limits().unwrap();
    assert_eq!(limits.input_bytes, 16_000_000);
    assert_eq!(limits.output_ids, 16_000_000);
}
