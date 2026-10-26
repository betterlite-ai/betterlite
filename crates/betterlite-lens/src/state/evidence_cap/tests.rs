#![cfg(test)]

use crate::models::finding::{Evidence, Role};
use crate::state::evidence_cap::merge_evidence;

fn evidence(execution_id: &str, role: Role) -> Evidence {
    Evidence {
        execution_id: execution_id.to_owned(),
        span_id: "span".to_owned(),
        quote: "quote".to_owned(),
        role,
    }
}

#[test]
fn a_repeated_key_keeps_its_place_and_takes_the_new_value() {
    let previous = [evidence("a", Role::Support), evidence("b", Role::Support)];
    let draft = [
        evidence("a", Role::Counterexample),
        evidence("c", Role::Support),
    ];
    let merged = merge_evidence(&previous, &draft);
    let expected = [
        evidence("a", Role::Counterexample),
        evidence("b", Role::Support),
        evidence("c", Role::Support),
    ];
    assert_eq!(merged, expected);
}

#[test]
fn only_the_newest_twenty_survive() {
    let previous: Vec<Evidence> = (0..15_u8)
        .map(|index| evidence(&format!("old-{index}"), Role::Support))
        .collect();
    let draft: Vec<Evidence> = (0..10_u8)
        .map(|index| evidence(&format!("new-{index}"), Role::Support))
        .collect();
    let merged = merge_evidence(&previous, &draft);
    assert_eq!(merged.len(), 20);
    assert_eq!(merged[..10], previous[5..]);
    assert_eq!(merged[10..], draft[..]);
}
