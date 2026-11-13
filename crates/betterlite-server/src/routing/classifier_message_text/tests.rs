#![cfg(test)]

use betterlite_config::sections::classifier_message_text as section;

use crate::routing::classifier_message_text::deployment_markers;

fn configured(open: &str, close: &str) -> Vec<section::MarkerPair> {
    vec![section::MarkerPair {
        open: open.to_owned(),
        close: close.to_owned(),
    }]
}

#[test]
fn a_deployment_without_markers_keeps_its_index() {
    let settings = section::Settings {
        deployments: vec![None, Some(configured("<open>", "</close>"))],
    };
    assert_eq!(deployment_markers(&settings, 0), None);
}

#[test]
fn configured_markers_borrow_the_section() {
    let settings = section::Settings {
        deployments: vec![Some(configured("<open>", "</close>"))],
    };
    let markers = deployment_markers(&settings, 0).unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].open, "<open>");
    assert_eq!(markers[0].close, "</close>");
}

#[test]
fn a_deployment_past_the_list_has_no_markers() {
    let settings = section::Settings {
        deployments: vec![Some(configured("<open>", "</close>"))],
    };
    assert_eq!(deployment_markers(&settings, 1), None);
}
