use super::*;

#[tokio::test]
async fn keeps_both_messages_of_a_fitting_span() {
    let normalized = vec![
        json!({"content": "a a a"}),
        json!({"content": "b b"}),
        json!({"content": words(7)}),
    ];
    let scores = vec![0.1_f64, 0.9_f64, 0.5_f64];
    let spans = vec![vec![0_usize, 1]];
    let selection = select_budget(&normalized, &scores, 6, &[], &spans).await;
    assert_eq!(selection.kept_indices, BTreeSet::from([0, 1]));
}

#[tokio::test]
async fn never_truncates_a_span_that_does_not_fit() {
    let normalized = vec![
        json!({"content": words(60)}),
        json!({"content": words(60)}),
        json!({"content": "c"}),
    ];
    let scores = vec![0.9_f64, 0.9_f64, 0.5_f64];
    let spans = vec![vec![0_usize, 1]];
    let selection = select_budget(&normalized, &scores, 100, &[], &spans).await;
    assert_eq!(selection.kept_indices, BTreeSet::from([2]));
    assert!(selection.truncated_overrides.is_empty());
}

#[tokio::test]
async fn scores_a_span_by_its_best_member() {
    let normalized = vec![
        json!({"content": "a"}),
        json!({"content": "b"}),
        json!({"content": "c c c"}),
    ];
    let scores = vec![0.1_f64, 0.95_f64, 0.5_f64];
    let spans = vec![vec![0_usize, 1]];
    let selection = select_budget(&normalized, &scores, 3, &[], &spans).await;
    assert_eq!(selection.kept_indices, BTreeSet::from([0, 1]));
}

#[tokio::test]
async fn a_protected_member_keeps_the_whole_span() {
    let messages = vec![
        json!({"role": "user", "content": "start"}),
        json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": "t1", "name": "lookup", "input": {},
             "cache_control": {"type": "ephemeral"}}
        ]}),
        json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": [{"type": "text", "text": "result"}]}
        ]}),
        json!({"role": "assistant", "content": "done"}),
        json!({"role": "user", "content": "now"}),
    ];
    let counter = fixture_counter(vec![90, 70]);
    let mut compress_request = request(&messages, 0, Some(0));
    compress_request.call_type = "anthropic_messages";
    let result = compress(&compress_request, &counter)
        .await
        .expect("compressed");
    assert_eq!(result.messages, messages);
    assert!(result.cache.is_empty());
}
