#![cfg(test)]
use super::cluster::consolidate;
use super::fixtures::{candidate, echo_model, observation};
use crate::analysis::ObservationKind;

#[tokio::test]
async fn drafts_a_title_cut_at_160_characters_and_a_kind_prefixed_hypothesis() {
    let (model, _calls) = echo_model();
    let prefix = "\u{e9}".repeat(165);
    let summary = format!("{prefix} trailing text that goes past the title cutoff");
    let batch = vec![observation(
        "a",
        ObservationKind::Pattern,
        &summary,
        vec!["r0"],
    )];
    let result = consolidate(batch, Vec::new(), &model).await.unwrap();
    assert_eq!(result.len(), 1);
    let expected_title: String = summary.chars().take(160).collect();
    assert_eq!(result[0].title, expected_title);
    assert_eq!(result[0].hypothesis, format!("pattern: {summary}"));
}

#[tokio::test]
async fn with_no_previous_candidates_keeps_every_incoming_candidate() {
    let (model, calls) = echo_model();
    let batch = vec![
        observation("a", ObservationKind::Issue, "first", vec!["r0"]),
        observation("b", ObservationKind::Issue, "second", vec!["r1"]),
    ];
    let result = consolidate(batch, Vec::new(), &model).await.unwrap();
    assert_eq!(*calls.lock().unwrap(), 1);
    let mut check_ids: Vec<_> = result
        .iter()
        .map(|candidate| candidate.check_id.clone())
        .collect();
    check_ids.sort();
    assert_eq!(check_ids, vec![String::from("a"), String::from("b")]);
}

#[tokio::test]
async fn rebinds_active_candidates_without_losing_or_duplicating_previous() {
    let (model, _calls) = echo_model();
    let first_batch = vec![observation(
        "a",
        ObservationKind::Issue,
        "first",
        vec!["r0"],
    )];
    let previous = consolidate(first_batch, Vec::new(), &model).await.unwrap();
    assert_eq!(previous.len(), 1);

    let second_batch = vec![observation(
        "b",
        ObservationKind::Issue,
        "second",
        vec!["r1"],
    )];
    let result = consolidate(second_batch, previous, &model).await.unwrap();

    let mut check_ids: Vec<_> = result
        .iter()
        .map(|candidate| candidate.check_id.clone())
        .collect();
    check_ids.sort();
    assert_eq!(check_ids, vec![String::from("a"), String::from("b")]);
    let mut execution_ids: Vec<_> = result
        .iter()
        .flat_map(|candidate| candidate.execution_ids.clone())
        .collect();
    execution_ids.sort();
    assert_eq!(execution_ids, vec![String::from("r0"), String::from("r1")]);
}

#[tokio::test]
async fn pages_the_previous_registry_without_losing_entries() {
    let (model, calls) = echo_model();
    let long_title = "x".repeat(160);
    let long_hypothesis = "x".repeat(2000);
    let previous: Vec<_> = (0_usize..8_usize)
        .map(|index| {
            candidate(
                &format!("check{index}"),
                ObservationKind::Issue,
                &long_title,
                &long_hypothesis,
                vec!["r0"],
            )
        })
        .collect();

    let final_batch = vec![observation(
        "checkN",
        ObservationKind::Issue,
        "final",
        vec!["r1"],
    )];
    let result = consolidate(final_batch, previous, &model).await.unwrap();

    assert!(
        *calls.lock().unwrap() > 1,
        "expected the oversized registry to span multiple pages"
    );
    assert_eq!(result.len(), 9);
    let mut check_ids: Vec<_> = result
        .iter()
        .map(|candidate| candidate.check_id.clone())
        .collect();
    check_ids.sort();
    let mut expected: Vec<String> = (0_usize..8_usize)
        .map(|index| format!("check{index}"))
        .collect();
    expected.push(String::from("checkN"));
    expected.sort();
    assert_eq!(check_ids, expected);
}
