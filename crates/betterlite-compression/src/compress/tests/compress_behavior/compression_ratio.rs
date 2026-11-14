use super::super::*;
use rstest::rstest;

#[rstest]
#[case::exact_fraction(1000_usize, 400_usize, 0.6_f64)]
#[case::rounds_to_four_places(3, 2, 0.3333)]
#[case::ties_round_to_even(32, 31, 0.0312)]
#[case::double_rounds_up(160, 7, 0.9563)]
#[case::double_rounds_down(160, 9, 0.9437)]
#[case::grows_when_stubs_are_larger(10, 15, -0.5)]
#[tokio::test]
async fn is_one_minus_compressed_over_original(
    #[case] original_total: usize,
    #[case] compressed_total: usize,
    #[case] expected: f64,
) {
    let messages = vec![
        json!({"role": "system", "content": "s"}),
        json!({"role": "user", "content": "u"}),
    ];
    let counter = fixture_counter(vec![original_total, compressed_total]);
    let result = compress(&request(&messages, 0, Some(0)), &counter)
        .await
        .expect("compressed");
    assert_eq!(result.compression_ratio.to_bits(), expected.to_bits());
}
