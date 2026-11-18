use rstest::rstest;

use super::support::{Snapshot, group};

#[test]
fn routing_group_returns_a_callable_group() {
    let snapshot = Snapshot::default().with_groups(vec![group("cheap", &["a", "b"])], &[], &[]);
    let recognition = snapshot.recognition();
    let found = recognition
        .routing_group("cheap")
        .expect("the group is callable");
    assert_eq!(found.group_name, "cheap");
    assert_eq!(found.models, vec!["a".to_owned(), "b".to_owned()]);
}

#[test]
fn routing_group_returns_none_when_a_deployment_shadows_the_name() {
    let snapshot = Snapshot::default()
        .with_deployment("cheap", "dep-1")
        .with_groups(vec![group("cheap", &["a"])], &[], &[]);
    assert!(snapshot.recognition().routing_group("cheap").is_none());
}

#[test]
fn routing_group_returns_none_when_an_alias_shadows_the_name() {
    let snapshot = Snapshot::default()
        .with_alias("cheap", "gpt-4o")
        .with_groups(vec![group("cheap", &["a"])], &[], &[]);
    assert!(snapshot.recognition().routing_group("cheap").is_none());
}

#[test]
fn routing_group_returns_none_for_an_unknown_name() {
    let snapshot = Snapshot::default().with_groups(vec![group("cheap", &["a"])], &[], &[]);
    assert!(snapshot.recognition().routing_group("expensive").is_none());
}

#[test]
fn routing_group_returns_none_when_the_index_marked_the_name_shadowed() {
    let snapshot = Snapshot::default().with_groups(vec![group("cheap", &["a"])], &["cheap"], &[]);
    assert!(snapshot.recognition().routing_group("cheap").is_none());
}

#[test]
fn routing_group_returns_none_without_groups() {
    assert!(
        Snapshot::default()
            .recognition()
            .routing_group("cheap")
            .is_none()
    );
}

#[test]
fn routing_group_keeps_a_group_named_like_an_alias_target() {
    let snapshot = Snapshot::default().with_alias("fast", "cheap").with_groups(
        vec![group("cheap", &["a"])],
        &[],
        &[],
    );
    assert!(snapshot.recognition().routing_group("cheap").is_some());
}

fn served() -> Snapshot {
    Snapshot::default()
        .with_deployment("gpt-4o", "dep-1")
        .with_alias("fast", "elsewhere")
        .with_groups(vec![group("cheap", &["gpt-4o"])], &[], &[])
}

#[rstest]
#[case::deployment_model_name("gpt-4o")]
#[case::deployment_id("dep-1")]
#[case::model_group_alias("fast")]
#[case::callable_routing_group("cheap")]
fn is_recognized_accepts_every_served_kind(#[case] model: &str) {
    assert!(served().recognition().is_recognized(model));
}

#[rstest]
#[case::unknown_name("unknown")]
#[case::empty_name("")]
#[case::prefix_of_a_deployment_name("gpt-4")]
#[case::alias_target("elsewhere")]
fn is_recognized_rejects_a_name_it_does_not_serve(#[case] model: &str) {
    assert!(!served().recognition().is_recognized(model));
}

#[test]
fn is_recognized_rejects_a_group_the_index_marked_shadowed() {
    let snapshot =
        Snapshot::default().with_groups(vec![group("cheap", &["gpt-4o"])], &["cheap"], &[]);
    assert!(!snapshot.recognition().is_recognized("cheap"));
}
