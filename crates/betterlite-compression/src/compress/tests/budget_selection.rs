use super::*;

#[tokio::test]
async fn stops_once_the_budget_is_exhausted() {
    let normalized = vec![json!({"content": words(10)}), json!({"content": null})];
    let scores = vec![0.9_f64, 0.5_f64];
    let selection = select_budget(&normalized, &scores, 10, &[], &[]).await;
    assert_eq!(selection.kept_indices, BTreeSet::from([0]));
}

#[tokio::test]
async fn keeps_highest_scoring_units_first() {
    let normalized = vec![
        json!({"content": words(10)}),
        json!({"content": words(10)}),
        json!({"content": words(10)}),
    ];
    let scores = vec![0.1_f64, 0.9_f64, 0.5_f64];
    let selection = select_budget(&normalized, &scores, 25, &[], &[]).await;
    assert_eq!(selection.kept_indices, BTreeSet::from([1, 2]));
}

#[tokio::test]
async fn tries_tied_singles_in_index_order() {
    let normalized = vec![
        json!({"content": words(10)}),
        json!({"content": words(10)}),
        json!({"content": words(10)}),
    ];
    let scores = vec![0.0_f64, 0.0_f64, 0.0_f64];
    let selection = select_budget(&normalized, &scores, 20, &[], &[]).await;
    assert_eq!(selection.kept_indices, BTreeSet::from([0, 1]));
}

#[tokio::test]
async fn tries_a_tied_single_before_a_span() {
    let normalized = vec![
        json!({"content": words(5)}),
        json!({"content": words(5)}),
        json!({"content": words(10)}),
    ];
    let scores = vec![0.0_f64, 0.0_f64, 0.0_f64];
    let spans = vec![vec![0_usize, 1]];
    let selection = select_budget(&normalized, &scores, 10, &[], &spans).await;
    assert_eq!(selection.kept_indices, BTreeSet::from([2]));
}

mod preserves_protected;
mod token_counting;
mod truncation;
