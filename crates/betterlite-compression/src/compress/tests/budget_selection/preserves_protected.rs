use super::super::*;

#[tokio::test]
async fn keeps_protected_indices_ahead_of_higher_scorers() {
    let normalized = vec![
        json!({"content": words(10)}),
        json!({"content": words(10)}),
        json!({"content": words(10)}),
    ];
    let scores = vec![0.1_f64, 0.99, 0.5];
    let selection = select_budget(&normalized, &scores, 10, &[0], &[]).await;
    assert_eq!(selection.kept_indices, BTreeSet::from([0]));
}
