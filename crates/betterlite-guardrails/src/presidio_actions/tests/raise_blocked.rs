#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::enums::PiiAction;

use super::shared::item;
use crate::presidio_actions::blocking::raise_if_blocked_entities;
use crate::presidio_actions::error::PresidioActionsError;

fn config(action: PiiAction, entity_type: &str) -> BTreeMap<String, PiiAction> {
    BTreeMap::from([(entity_type.to_owned(), action)])
}

#[test]
fn a_blocked_entity_raises_with_the_guardrail_name() {
    let error = raise_if_blocked_entities(
        &[item("PERSON", 0, 5)],
        &config(PiiAction::Block, "PERSON"),
        "presidio-global",
    )
    .unwrap_err();
    let PresidioActionsError::BlockedEntity {
        entity_type,
        guardrail_name,
    } = error
    else {
        panic!("expected a blocked entity error");
    };
    assert_eq!(entity_type, "PERSON");
    assert_eq!(guardrail_name, "presidio-global");
}

#[test]
fn the_blocked_entity_error_has_status_400() {
    let error = raise_if_blocked_entities(
        &[item("EMAIL_ADDRESS", 0, 5)],
        &config(PiiAction::Block, "EMAIL_ADDRESS"),
        "presidio-global",
    )
    .unwrap_err();
    assert_eq!(error.status(), 400);
}

#[test]
fn a_mask_action_does_not_raise() {
    raise_if_blocked_entities(
        &[item("PERSON", 0, 5)],
        &config(PiiAction::Mask, "PERSON"),
        "presidio-global",
    )
    .unwrap();
}

#[test]
fn an_empty_config_blocks_nothing() {
    raise_if_blocked_entities(&[item("PERSON", 0, 5)], &BTreeMap::new(), "presidio-global")
        .unwrap();
}

#[test]
fn detections_without_an_entity_type_are_skipped() {
    let mut detection = item("PERSON", 0, 5);
    detection.entity_type = None;
    raise_if_blocked_entities(
        &[detection],
        &config(PiiAction::Block, "PERSON"),
        "presidio-global",
    )
    .unwrap();
}

#[test]
fn any_blocked_detection_in_the_list_raises() {
    let error = raise_if_blocked_entities(
        &[item("PERSON", 0, 5), item("US_SSN", 6, 17)],
        &config(PiiAction::Block, "US_SSN"),
        "presidio-global",
    )
    .unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::BlockedEntity { entity_type, .. } if entity_type == "US_SSN"
    ));
}
