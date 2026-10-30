use crate::input_sequence_length::get_input_sequence_length_bucket;
#[test]
fn handles_boundaries() {
    assert_eq!(get_input_sequence_length_bucket(Some(999)), "0-1k");
    assert_eq!(get_input_sequence_length_bucket(Some(1_000)), "1k-4k");
    assert_eq!(get_input_sequence_length_bucket(Some(4_000)), "4k-16k");
    assert_eq!(get_input_sequence_length_bucket(Some(-1)), "unknown");
}
