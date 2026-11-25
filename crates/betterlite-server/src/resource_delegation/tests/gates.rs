use crate::resource_delegation::gates::{validate_search_tools, validate_vector_stores};

use super::fixtures::{ceiling_grants, gate_detail, json, team_fields};

#[test]
fn vector_personal_gate_only() {
    let denied = json(r#"{"vector_stores": ["v-b", "v-a", "v-b", ""]}"#);
    let error = validate_vector_stores(Some(&denied), None, false).unwrap_err();
    assert_eq!(
        gate_detail(error),
        "Key is not in a team. Vector stores cannot be assigned to personal keys by non-admin callers. Disallowed vector stores: ['v-a', 'v-b']."
    );
    validate_vector_stores(Some(&denied), None, true).unwrap();
    let permissionless_team = team_fields("team-1", &[], None);
    validate_vector_stores(Some(&denied), Some(&permissionless_team), false).unwrap();
    let team = team_fields("team-1", &[], Some(ceiling_grants(None, None)));
    validate_vector_stores(
        Some(&json(r#"{"vector_stores": ["any-store"]}"#)),
        Some(&team),
        false,
    )
    .unwrap();
    validate_vector_stores(Some(&json(r#"{"vector_stores": ["", ""]}"#)), None, false).unwrap();
    validate_vector_stores(None, None, false).unwrap();
}

#[test]
fn search_personal_duplicates() {
    let denied = json(r#"{"search_tools": ["b", "a", "b", ""]}"#);
    let error = validate_search_tools(Some(&denied), None, false).unwrap_err();
    assert_eq!(
        gate_detail(error),
        "Key is not in a team. search_tools cannot be assigned to personal keys by non-admin callers. Disallowed search tools: ['a', 'b', 'b']."
    );
    validate_search_tools(Some(&denied), None, true).unwrap();
    let open_team = team_fields("team-1", &[], Some(ceiling_grants(None, None)));
    validate_search_tools(Some(&denied), Some(&open_team), false).unwrap();
    let permissionless_team = team_fields("team-1", &[], None);
    validate_search_tools(Some(&denied), Some(&permissionless_team), false).unwrap();
}

#[test]
fn search_nonempty_ceiling() {
    let team = team_fields(
        "team-1",
        &[],
        Some(ceiling_grants(None, Some(&["a", "a", "b"]))),
    );
    let expected = "Key requests search tools not allowed by team 'team-1': ['c']. Team allows: ['a', 'a', 'b'].";
    let error = validate_search_tools(
        Some(&json(r#"{"search_tools": ["c", "a"]}"#)),
        Some(&team),
        false,
    )
    .unwrap_err();
    assert_eq!(gate_detail(error), expected);
    validate_search_tools(
        Some(&json(r#"{"search_tools": ["a", "a"]}"#)),
        Some(&team),
        false,
    )
    .unwrap();
    let admin_error =
        validate_search_tools(Some(&json(r#"{"search_tools": ["c"]}"#)), Some(&team), true)
            .unwrap_err();
    assert_eq!(gate_detail(admin_error), expected);
    let repeated_error = validate_search_tools(
        Some(&json(r#"{"search_tools": ["c", "c"]}"#)),
        Some(&team),
        false,
    )
    .unwrap_err();
    assert_eq!(gate_detail(repeated_error), expected);
    let empty_ceiling = team_fields("team-1", &[], Some(ceiling_grants(None, Some(&[]))));
    validate_search_tools(
        Some(&json(r#"{"search_tools": ["z"]}"#)),
        Some(&empty_ceiling),
        false,
    )
    .unwrap();
}
