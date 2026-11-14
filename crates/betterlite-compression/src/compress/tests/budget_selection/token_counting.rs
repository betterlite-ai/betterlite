use super::super::*;
use rstest::rstest;

#[rstest]
#[case::fits_exactly(110_usize, true)]
#[case::one_token_short(109, false)]
#[tokio::test]
async fn counts_initial_kept_indices_against_the_target(#[case] target: usize, #[case] kept: bool) {
    let normalized = vec![json!({"content": words(60)}), json!({"content": words(50)})];
    let scores = vec![0.9_f64, 0.8_f64];
    let selection = select_budget(&normalized, &scores, target, &[0], &[]).await;
    assert_eq!(selection.kept_indices.contains(&1), kept);
    assert!(selection.kept_indices.contains(&0));
}
