use rstest::rstest;

use super::support::cached_entry;
use crate::embedding_merge::cached::extract_cached_model;
use crate::types::embedding::CachedEmbedding;

enum Slot {
    Uncached,
    Unnamed,
    Named(&'static str),
}

fn results(slots: &[Slot]) -> Vec<Option<CachedEmbedding>> {
    slots
        .iter()
        .map(|slot| match slot {
            Slot::Uncached => None,
            Slot::Unnamed => Some(cached_entry(Some("a"), Some(1))),
            Slot::Named(name) => Some(CachedEmbedding {
                model: Some((*name).to_owned()),
                ..cached_entry(Some("a"), Some(1))
            }),
        })
        .collect()
}

#[rstest]
#[case::empty_list(&[], None)]
#[case::only_uncached_positions(&[Slot::Uncached, Slot::Uncached], None)]
#[case::entry_without_a_model(&[Slot::Unnamed], None)]
#[case::entry_with_an_empty_model(&[Slot::Named("")], None)]
#[case::first_entry_with_a_model(&[Slot::Named("m1"), Slot::Named("m2")], Some("m1"))]
#[case::uncached_positions_are_skipped(&[Slot::Uncached, Slot::Named("m1")], Some("m1"))]
#[case::entries_without_a_model_are_skipped(&[Slot::Unnamed, Slot::Named("m2")], Some("m2"))]
#[case::empty_models_are_skipped(&[Slot::Named(""), Slot::Named("m2")], Some("m2"))]
fn finds_the_first_entry_that_names_a_model(
    #[case] slots: &[Slot],
    #[case] expected: Option<&str>,
) {
    assert_eq!(extract_cached_model(&results(slots)).as_deref(), expected);
}
