use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::Subscriber;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::sso_mappings::literal::LiteralError;
use crate::sso_mappings::load::{load_role_mappings, load_team_mappings};
use crate::sso_mappings::roles::RoleMappingError;
use crate::sso_types::records::TeamMappings;
use crate::user_roles::UserRole;

use super::support::{FakeRows, Stored, role_mapping, sources};

const STORED_ROLE_MAPPING: &str = r#"{"role_mappings": {"provider": "generic", "group_claim": "groups", "roles": {"proxy_admin": ["admins"]}}}"#;

#[derive(Clone, Default)]
struct DebugCount(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for DebugCount {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::DEBUG {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn counted() -> (DebugCount, tracing::subscriber::DefaultGuard) {
    let count = DebugCount::default();
    let subscriber = tracing_subscriber::registry().with(count.clone());
    (count, set_default(subscriber))
}

#[tokio::test]
async fn the_stored_mapping_is_used_when_the_environment_is_absent() {
    let loaded =
        load_role_mappings(&FakeRows(Stored::Text(STORED_ROLE_MAPPING)), &sources(&[])).await;
    assert_eq!(
        loaded,
        Ok(Some(role_mapping(
            "groups",
            None,
            vec![(UserRole::ProxyAdmin, vec!["admins"])]
        )))
    );
}

#[tokio::test]
async fn the_environment_override_wins_over_the_stored_mapping() {
    let pairs = [
        (
            "GENERIC_ROLE_MAPPINGS_ROLES",
            "{'internal_user': ['users']}",
        ),
        ("GENERIC_ROLE_MAPPINGS_GROUP_CLAIM", "roles"),
        ("GENERIC_ROLE_MAPPINGS_DEFAULT_ROLE", "internal_user_viewer"),
    ];
    let loaded = load_role_mappings(
        &FakeRows(Stored::Text(STORED_ROLE_MAPPING)),
        &sources(&pairs),
    )
    .await;
    assert_eq!(
        loaded,
        Ok(Some(role_mapping(
            "roles",
            Some(UserRole::InternalUserViewer),
            vec![(UserRole::InternalUser, vec!["users"])]
        )))
    );
}

#[tokio::test]
async fn a_non_dictionary_override_keeps_the_stored_mapping() {
    let pairs = [("GENERIC_ROLE_MAPPINGS_ROLES", "['proxy_admin']")];
    let loaded = load_role_mappings(
        &FakeRows(Stored::Text(STORED_ROLE_MAPPING)),
        &sources(&pairs),
    )
    .await;
    assert_eq!(
        loaded,
        Ok(Some(role_mapping(
            "groups",
            None,
            vec![(UserRole::ProxyAdmin, vec!["admins"])]
        )))
    );
}

#[tokio::test]
async fn a_group_claim_without_roles_does_not_override() {
    let pairs = [("GENERIC_ROLE_MAPPINGS_GROUP_CLAIM", "roles")];
    let loaded = load_role_mappings(
        &FakeRows(Stored::Text(STORED_ROLE_MAPPING)),
        &sources(&pairs),
    )
    .await;
    assert_eq!(
        loaded,
        Ok(Some(role_mapping(
            "groups",
            None,
            vec![(UserRole::ProxyAdmin, vec!["admins"])]
        )))
    );
}

#[tokio::test]
async fn a_syntax_error_in_the_override_is_not_swallowed() {
    let pairs = [("GENERIC_ROLE_MAPPINGS_ROLES", "{'proxy_admin': ['admins']")];
    let loaded = load_role_mappings(
        &FakeRows(Stored::Text(STORED_ROLE_MAPPING)),
        &sources(&pairs),
    )
    .await;
    assert_eq!(
        loaded,
        Err(RoleMappingError::Literal {
            source: LiteralError::Syntax
        })
    );
}

#[tokio::test]
async fn a_dictionary_override_without_a_group_claim_fails() {
    let pairs = [("GENERIC_ROLE_MAPPINGS_ROLES", "{'proxy_admin': ['admins']}")];
    let loaded = load_role_mappings(&FakeRows(Stored::Absent), &sources(&pairs)).await;
    assert_eq!(loaded, Err(RoleMappingError::MissingGroupClaim));
}

#[tokio::test]
async fn a_stored_mapping_without_roles_has_no_roles() {
    let stored = r#"{"role_mappings": {"provider": "generic", "group_claim": "groups"}}"#;
    let loaded = load_role_mappings(&FakeRows(Stored::Text(stored)), &sources(&[])).await;
    assert_eq!(loaded, Ok(Some(role_mapping("groups", None, Vec::new()))));
}

#[tokio::test]
async fn an_unreadable_row_leaves_the_mapping_absent_and_logs_at_debug() {
    let (count, _guard) = counted();
    let loaded = load_role_mappings(&FakeRows(Stored::Unreadable), &sources(&[])).await;
    assert_eq!(loaded, Ok(None));
    assert_eq!(count.0.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn a_row_that_is_not_settings_json_leaves_the_mapping_absent_and_logs_at_debug() {
    let (count, _guard) = counted();
    let loaded = load_role_mappings(&FakeRows(Stored::Text("not json")), &sources(&[])).await;
    assert_eq!(loaded, Ok(None));
    assert_eq!(count.0.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn a_stored_role_that_is_not_a_litellm_role_leaves_the_mapping_absent() {
    let stored = r#"{"role_mappings": {"provider": "generic", "group_claim": "groups", "roles": {"wizard": ["admins"]}}}"#;
    let (count, _guard) = counted();
    let loaded = load_role_mappings(&FakeRows(Stored::Text(stored)), &sources(&[])).await;
    assert_eq!(loaded, Ok(None));
    assert_eq!(count.0.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn stored_keys_the_read_does_not_use_are_ignored() {
    let stored = r#"{"google_client_id": "client", "role_mappings": null}"#;
    let loaded = load_role_mappings(&FakeRows(Stored::Text(stored)), &sources(&[])).await;
    assert_eq!(loaded, Ok(None));
}

#[tokio::test]
async fn the_override_is_used_when_the_row_cannot_be_read() {
    let pairs = [
        ("GENERIC_ROLE_MAPPINGS_ROLES", "{'proxy_admin': []}"),
        ("GENERIC_ROLE_MAPPINGS_GROUP_CLAIM", "groups"),
    ];
    let (count, _guard) = counted();
    let loaded = load_role_mappings(&FakeRows(Stored::Unreadable), &sources(&pairs)).await;
    assert_eq!(
        loaded,
        Ok(Some(role_mapping(
            "groups",
            None,
            vec![(UserRole::ProxyAdmin, Vec::new())]
        )))
    );
    assert_eq!(count.0.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn the_team_mapping_is_read_from_the_row() {
    let stored = r#"{"team_mappings": {"team_ids_jwt_field": "teams"}}"#;
    let loaded = load_team_mappings(&FakeRows(Stored::Text(stored))).await;
    assert_eq!(
        loaded,
        Some(TeamMappings {
            team_ids_jwt_field: Some("teams".to_owned())
        })
    );
}

#[tokio::test]
async fn a_row_without_a_team_mapping_leaves_it_absent() {
    assert_eq!(
        load_team_mappings(&FakeRows(Stored::Text(STORED_ROLE_MAPPING))).await,
        None
    );
    assert_eq!(load_team_mappings(&FakeRows(Stored::Absent)).await, None);
}

#[tokio::test]
async fn a_row_that_is_not_settings_json_leaves_the_team_mapping_absent() {
    assert_eq!(
        load_team_mappings(&FakeRows(Stored::Text("[]"))).await,
        None
    );
}
