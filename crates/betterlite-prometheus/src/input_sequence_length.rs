pub const UNKNOWN_INPUT_SEQUENCE_LENGTH: &str = "unknown";

#[must_use]
pub const fn get_input_sequence_length_bucket(input: Option<i64>) -> &'static str {
    let Some(tokens) = input else {
        return UNKNOWN_INPUT_SEQUENCE_LENGTH;
    };
    if tokens < 0 {
        return UNKNOWN_INPUT_SEQUENCE_LENGTH;
    }
    match tokens {
        0..1_000 => "0-1k",
        1_000..4_000 => "1k-4k",
        4_000..16_000 => "4k-16k",
        16_000..64_000 => "16k-64k",
        _ => "64k+",
    }
}
