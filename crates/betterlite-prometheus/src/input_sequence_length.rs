pub const UNKNOWN_INPUT_SEQUENCE_LENGTH: &str = "unknown";
pub const INPUT_SEQUENCE_LENGTH_BUCKETS: [(Option<i64>, &str); 5] = [
    (Some(1_000), "0-1k"),
    (Some(4_000), "1k-4k"),
    (Some(16_000), "4k-16k"),
    (Some(64_000), "16k-64k"),
    (None, "64k+"),
];

#[must_use]
pub fn sequence_length_bucket(input: Option<i64>) -> &'static str {
    let Some(tokens) = input.filter(|value| *value >= 0) else {
        return UNKNOWN_INPUT_SEQUENCE_LENGTH;
    };
    INPUT_SEQUENCE_LENGTH_BUCKETS
        .iter()
        .find(|(upper, _)| upper.is_none_or(|bound| tokens < bound))
        .map_or(UNKNOWN_INPUT_SEQUENCE_LENGTH, |(_, label)| *label)
}
