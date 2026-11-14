use std::iter::repeat_with;

use super::super::*;
use rstest::rstest;

fn long_message() -> Value {
    let lines = repeat_with(|| {
        (0_usize..10_usize)
            .map(|word| format!("w{word}"))
            .collect::<Vec<String>>()
            .join(" ")
    })
    .take(20)
    .collect::<Vec<String>>()
    .join("\n");
    json!({"role": "user", "content": lines})
}

#[rstest]
#[case::hundred_tokens_left(100_usize, true)]
#[case::ninety_nine_tokens_left(99, false)]
#[tokio::test]
async fn truncates_only_with_enough_budget_left(#[case] target: usize, #[case] truncated: bool) {
    let message = long_message();
    let normalized = vec![message.clone()];
    let scores = vec![0.9_f64];
    let selection = select_budget(&normalized, &scores, target, &[], &[]).await;
    assert_eq!(selection.kept_indices.contains(&0), truncated);
    if truncated {
        let expected = truncate_message(&message, target);
        assert_ne!(expected["content"], message["content"]);
        assert_eq!(selection.truncated_overrides.get(&0), Some(&expected));
    } else {
        assert!(selection.truncated_overrides.is_empty());
    }
}

#[tokio::test]
async fn never_truncates_a_tool_exchange() {
    let normalized = vec![json!({"content": words(60)}), json!({"content": words(60)})];
    let scores = vec![0.9_f64, 0.9_f64];
    let spans = vec![vec![0_usize, 1]];
    let selection = select_budget(&normalized, &scores, 100, &[], &spans).await;
    assert!(selection.kept_indices.is_empty());
    assert!(selection.truncated_overrides.is_empty());
}

#[tokio::test]
async fn assembles_the_truncated_message() {
    let messages = vec![
        json!({"role": "system", "content": "s"}),
        json!({"role": "user", "content": words(150)}),
        json!({"role": "user", "content": "last"}),
    ];
    let counter = fixture_counter(vec![40, 20]);
    let result = compress(&request(&messages, 0, Some(120)), &counter)
        .await
        .expect("compressed");
    assert_eq!(result.messages[1], truncate_message(&messages[1], 118));
    assert!(result.cache.is_empty());
}

#[tokio::test]
async fn never_truncates_a_single_index_span() {
    let normalized = vec![json!({"content": words(150)})];
    let scores = vec![0.9_f64];
    let spans = vec![vec![0_usize]];
    let selection = select_budget(&normalized, &scores, 100, &[], &spans).await;
    assert!(selection.kept_indices.is_empty());
    assert!(selection.truncated_overrides.is_empty());
}

async fn select_with_counter(
    normalized: &[Value],
    scores: &[f64],
    target: usize,
    counter: &FakeCounter,
) -> BudgetSelection {
    select_kept_indices_for_budget(
        &BudgetSelectionRequest {
            normalized_messages: normalized,
            original_messages: normalized,
            combined_scores: scores,
            compression_target: target,
            model: MODEL,
            initial_kept_indices: &[],
            tool_exchange_spans: &[],
        },
        counter,
    )
    .await
    .expect("selected")
}

#[tokio::test]
async fn counts_the_truncated_message_against_the_budget() {
    let normalized = vec![long_message(), json!({"content": words(5)})];
    let scores = vec![0.9_f64, 0.1_f64];
    let expected = truncate_message(&normalized[0], 120);
    let expected_text = expected["content"].as_str().expect("truncated text");
    // the fake word count puts the truncated text over the target, so the next message cannot fit
    let full_counter = fixture_counter(Vec::new());
    let full_selection = select_with_counter(&normalized, &scores, 120, &full_counter).await;
    assert!(full_selection.kept_indices.contains(&0));
    assert!(!full_selection.kept_indices.contains(&1));
    // a truncated count below the target leaves 20 tokens, enough for the five-token message
    let mut counter = fixture_counter(Vec::new());
    counter.text_overrides.insert(expected_text.to_owned(), 100);
    let selection = select_with_counter(&normalized, &scores, 120, &counter).await;
    assert_eq!(selection.kept_indices, BTreeSet::from([0, 1]));
    let calls = counter.calls.lock().expect("recorded calls").clone();
    assert!(calls.contains(&CountCall::Text {
        model: MODEL.to_owned(),
        text: expected_text.to_owned(),
    }));
}
