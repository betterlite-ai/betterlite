#![cfg(test)]

use super::{Parameters, parameters};
use crate::models::settings::{MetadataFilter, Scope};

#[test]
fn builds_parameters_for_a_single_team() {
    let scope = Scope {
        team_id: "team-a".to_owned(),
        api_key_hash: "hash-1".to_owned(),
        all_teams: false,
    };
    let built = parameters(&scope, &[]);
    assert_eq!(
        built,
        Parameters {
            all_teams: 0,
            team: "team-a".to_owned(),
            key_hash: "hash-1".to_owned(),
            filter_keys: vec![],
            filter_values: vec![],
        }
    );
}

#[test]
fn builds_parameters_for_all_teams() {
    let scope = Scope {
        team_id: String::new(),
        api_key_hash: "hash-1".to_owned(),
        all_teams: true,
    };
    let built = parameters(&scope, &[]);
    assert_eq!(built.all_teams, 1);
}

#[test]
fn preserves_filter_order() {
    let scope = Scope::default();
    let filters = vec![
        MetadataFilter {
            key: "k1".to_owned(),
            value: "v1".to_owned(),
        },
        MetadataFilter {
            key: "k2".to_owned(),
            value: "v2".to_owned(),
        },
    ];
    let built = parameters(&scope, &filters);
    assert_eq!(built.filter_keys, vec!["k1".to_owned(), "k2".to_owned()]);
    assert_eq!(built.filter_values, vec!["v1".to_owned(), "v2".to_owned()]);
}
