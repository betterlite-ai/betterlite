#![cfg(test)]

use betterlite_db::database_availability::message::TRANSIENT_MESSAGE;
use betterlite_db::database_availability::policy::AllowRequestsOnOutage;
use tracing::Level;

use super::alias::get_org_object_by_alias;
use super::error::{OrgFailure, OrgLookupError, as_auth_failure, is_outage, proxy_exception};
use super::last_known::get_org_object_for_request;
use super::organization::{OrgId, get_org_object};
use super::project::get_project_object;
use crate::auth_errors::conversion::AuthFailure;

mod postgres;
mod support;

use support::{Levels, assert_events, cached_project, org_row, project_row, with_budget, world};

fn org_id(org_id: &str) -> OrgId {
    OrgId::Text(org_id.to_owned())
}

#[tokio::test]
async fn org_last_known_on_outage() {
    let world = world();
    world.hold_last_known("o1", org_row("o1", None, "b1"));
    world.script("outage org");
    let lookup = support::lookup(&world);
    let org = get_org_object_for_request(&lookup, "o1", &AllowRequestsOnOutage::Boolean(false))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(org.organization_id.as_deref(), Some("o1"));
    assert_events(
        &world,
        &[
            "read org_id:o1:with_budget local=false",
            "read org_id:o1:with_budget:last_known local=false",
        ],
    );
}

#[tokio::test]
async fn project_not_found() {
    let world = world();
    let lookup = support::lookup(&world);
    let project = get_project_object(&lookup, "p1", 1.0_f64).await.unwrap();
    assert!(project.is_none());
    assert_events(&world, &["read project_id:p1", "project p1"]);
    assert!(world.cached_project("project_id:p1").is_none());
}

#[tokio::test]
async fn org_alias() {
    let world = world();
    world.seed_org(org_row("o1", Some("acme"), "b1"));
    let lookup = support::lookup(&world);
    let org = get_org_object_by_alias(&lookup, "acme").await.unwrap();
    assert_eq!(org.organization_id.as_deref(), Some("o1"));
    assert!(
        org.litellm_budget_table.is_none(),
        "the alias query asks for no include"
    );
    assert_events(
        &world,
        &[
            "read org_alias:acme local=false",
            "alias acme",
            "write org_alias:acme ttl=5",
            "write org_id:o1 ttl=5",
        ],
    );
    assert_eq!(
        world
            .cached_org("org_alias:acme")
            .map(|cached| cached.organization_id),
        Some(Some("o1".to_owned()))
    );
    assert_eq!(
        world
            .cached_org("org_id:o1")
            .map(|cached| cached.organization_id),
        Some(Some("o1".to_owned()))
    );
}

#[tokio::test]
async fn alias_no_db_before_cache() {
    let world = world();
    world.hold_org("org_alias:acme", org_row("cached", Some("acme"), "b1"));
    let lookup = support::without_database(&world);
    let error = get_org_object_by_alias(&lookup, "acme").await.unwrap_err();
    assert!(matches!(&error, OrgLookupError::NoDatabase));
    assert_eq!(
        error.to_string(),
        "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"
    );
    assert_eq!(error.http(), None);
    assert_events(&world, &[]);
}

#[tokio::test]
async fn alias_cache_hit() {
    let world = world();
    world.hold_org("org_alias:acme", org_row("cached", Some("acme"), "b1"));
    let lookup = support::lookup(&world);
    let org = get_org_object_by_alias(&lookup, "acme").await.unwrap();
    assert_eq!(org.organization_id.as_deref(), Some("cached"));
    assert_events(&world, &["read org_alias:acme local=false"]);
}

#[tokio::test]
async fn alias_zero_404() {
    let world = world();
    let lookup = support::lookup(&world);
    let error = get_org_object_by_alias(&lookup, "gone").await.unwrap_err();
    assert!(matches!(&error, OrgLookupError::AliasNotFound { .. }));
    assert_eq!(error.http(), Some(404));
    assert_eq!(
        error.to_string(),
        "Organization with alias 'gone' doesn't exist in db. Create organization via `/organization/new` call."
    );
    assert_eq!(
        as_auth_failure(&error),
        AuthFailure::Http {
            detail: error.to_string(),
            param: None,
            status_code: 404,
            invalid_virtual_key: false,
        }
    );
    assert_events(&world, &["read org_alias:gone local=false", "alias gone"]);
    assert!(world.cached_org("org_alias:gone").is_none());
}

#[tokio::test]
async fn alias_multiple_400() {
    let world = world();
    world.seed_org(org_row("o1", Some("shared"), "b1"));
    world.seed_org(org_row("o2", Some("shared"), "b1"));
    let lookup = support::lookup(&world);
    let error = get_org_object_by_alias(&lookup, "shared")
        .await
        .unwrap_err();
    assert!(matches!(&error, OrgLookupError::AliasAmbiguous { .. }));
    assert_eq!(error.http(), Some(400));
    assert_eq!(
        error.to_string(),
        "Multiple organizations found with alias 'shared'. Please use org_id_jwt_field instead or ensure organization aliases are unique."
    );
    assert_events(
        &world,
        &["read org_alias:shared local=false", "alias shared"],
    );
    assert!(world.lock().orgs.is_empty(), "an ambiguous alias is cached");
}

#[tokio::test]
async fn alias_cache_write_order() {
    let world = world();
    world.seed_org(with_budget(org_row("o1", Some("acme"), "b1"), "10.0"));
    let lookup = support::lookup(&world);
    get_org_object_by_alias(&lookup, "acme").await.unwrap();
    assert_events(
        &world,
        &[
            "read org_alias:acme local=false",
            "alias acme",
            "write org_alias:acme ttl=5",
            "write org_id:o1 ttl=5",
        ],
    );
    let aliased = serde_json::to_value(world.cached_org("org_alias:acme").unwrap()).unwrap();
    let by_id = serde_json::to_value(world.cached_org("org_id:o1").unwrap()).unwrap();
    assert_eq!(aliased, by_id);
}

#[tokio::test]
async fn alias_write_failure_500() {
    let world = world();
    world.seed_org(org_row("o1", Some("acme"), "b1"));
    world.script("write org_id:o1");
    let levels = Levels::default();
    let _guard = levels.install();
    let lookup = support::lookup(&world);
    let error = get_org_object_by_alias(&lookup, "acme").await.unwrap_err();
    assert!(matches!(&error, OrgLookupError::AliasFailure { .. }));
    assert_eq!(error.http(), Some(500));
    assert_eq!(
        error.to_string(),
        "Error looking up organization by alias 'acme': the organization cache failed"
    );
    assert!(
        world.cached_org("org_alias:acme").is_some(),
        "the first write stays when the second one fails"
    );
    assert!(world.cached_org("org_id:o1").is_none());
    assert_events(
        &world,
        &[
            "read org_alias:acme local=false",
            "alias acme",
            "write org_alias:acme ttl=5",
        ],
    );
    assert_eq!(levels.seen(), vec![Level::ERROR]);
}

#[tokio::test]
async fn alias_read_failure_propagates() {
    let world = world();
    world.script("read org_alias:acme");
    let lookup = support::lookup(&world);
    let error = get_org_object_by_alias(&lookup, "acme").await.unwrap_err();
    assert!(matches!(&error, OrgLookupError::CacheRead(_)));
    assert_eq!(error.http(), None);
    assert_events(&world, &[]);
    assert!(
        !world
            .events()
            .iter()
            .any(|event| event.starts_with("alias "))
    );
    assert!(world.lock().orgs.is_empty());
}

#[tokio::test]
async fn id_nonstring_after_db_guard() {
    let world = world();
    let unwired = support::without_database(&world);
    let guarded = get_org_object(&unwired, OrgId::NotString, false).await;
    assert!(matches!(guarded, Err(OrgLookupError::NoDatabase)));
    let wired = support::lookup(&world);
    let absent = get_org_object(&wired, OrgId::NotString, false)
        .await
        .unwrap();
    assert!(absent.is_none());
    assert_events(&world, &[]);
}

#[tokio::test]
async fn budget_cache_partition() {
    let world = world();
    world.seed_org(with_budget(org_row("o1", None, "b1"), "10.0"));
    let lookup = support::lookup(&world);
    let with = get_org_object(&lookup, org_id("o1"), true)
        .await
        .unwrap()
        .unwrap();
    assert!(with.litellm_budget_table.is_some());
    let plain = get_org_object(&lookup, org_id("o1"), false)
        .await
        .unwrap()
        .unwrap();
    assert!(plain.litellm_budget_table.is_none());
    assert_events(
        &world,
        &[
            "read org_id:o1:with_budget local=false",
            "org o1 budget=true",
            "write org_id:o1:with_budget ttl=5",
            "read org_id:o1 local=false",
            "org o1 budget=false",
            "write org_id:o1 ttl=5",
        ],
    );
    world.clear_events();
    let again = get_org_object(&lookup, org_id("o1"), true)
        .await
        .unwrap()
        .unwrap();
    assert!(
        again.litellm_budget_table.is_some(),
        "the budget hit is kept"
    );
    assert_events(&world, &["read org_id:o1:with_budget local=false"]);
}

#[tokio::test]
async fn id_missing_is_typed_absence() {
    let world = world();
    world.hold_last_known("o1", org_row("stale", None, "b1"));
    let lookup = support::lookup(&world);
    let error = get_org_object(&lookup, org_id("o1"), true)
        .await
        .unwrap_err();
    assert!(matches!(&error, OrgLookupError::NotFound { org_id } if org_id == "o1"));
    assert_eq!(
        error.to_string(),
        "Organization doesn't exist in db. Organization=o1. Create organization via `/organization/new` call."
    );
    assert_eq!(error.http(), None);
    assert!(!is_outage(&error));
    assert_eq!(
        as_auth_failure(&error),
        AuthFailure::Other {
            message: error.to_string(),
            param: None,
        }
    );
    let absent = get_org_object_for_request(&lookup, "o1", &AllowRequestsOnOutage::Unsupported)
        .await
        .unwrap();
    assert!(absent.is_none());
    assert!(
        !world
            .events()
            .iter()
            .any(|event| event.contains("last_known"))
    );
}

#[tokio::test]
async fn id_fault_propagates() {
    let world = world();
    world.script("fault org");
    let lookup = support::lookup(&world);
    let error = get_org_object(&lookup, org_id("o1"), true)
        .await
        .unwrap_err();
    assert!(matches!(&error, OrgLookupError::Failure(_)));
    assert!(!is_outage(&error));
    assert_eq!(error.http(), None);
    let exception = proxy_exception(&error);
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.code, "401");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert!(exception.message.starts_with("Authentication Error, "));
}

#[tokio::test]
async fn confirmed_absence_ignores_stale() {
    let world = world();
    world.hold_last_known("o1", org_row("stale", None, "b1"));
    let lookup = support::lookup(&world);
    let org = get_org_object_for_request(&lookup, "o1", &AllowRequestsOnOutage::Boolean(true))
        .await
        .unwrap();
    assert!(org.is_none());
    assert!(
        !world
            .events()
            .iter()
            .any(|event| event.contains("last_known"))
    );
    assert_events(
        &world,
        &[
            "read org_id:o1:with_budget local=false",
            "org o1 budget=true",
        ],
    );
}

#[tokio::test]
async fn nonoutage_debug_drop() {
    let world = world();
    world.script("fault org");
    let levels = Levels::default();
    let _guard = levels.install();
    let lookup = support::lookup(&world);
    let org = get_org_object_for_request(&lookup, "o1", &AllowRequestsOnOutage::Boolean(false))
        .await
        .unwrap();
    assert!(org.is_none());
    assert!(
        !world
            .events()
            .iter()
            .any(|event| event.contains("last_known"))
    );
    assert_eq!(levels.seen(), vec![Level::DEBUG]);
}

#[tokio::test]
async fn outage_stale_before_policy() {
    let world = world();
    world.hold_last_known("o1", org_row("o1", None, "b1"));
    world.script("outage org");
    let lookup = support::lookup(&world);
    let org = get_org_object_for_request(&lookup, "o1", &AllowRequestsOnOutage::Unsupported)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(org.organization_id.as_deref(), Some("o1"));
}

#[tokio::test]
async fn outage_no_stale_policy_matrix() {
    let world = world();
    world.script("outage org");
    let lookup = support::lookup(&world);
    let text_allow = AllowRequestsOnOutage::Text(" TRUE ".to_owned());
    let allowed = get_org_object_for_request(&lookup, "o1", &text_allow)
        .await
        .unwrap();
    assert!(allowed.is_none());

    let denied = get_org_object_for_request(&lookup, "o1", &AllowRequestsOnOutage::Boolean(false))
        .await
        .unwrap_err();
    assert!(is_outage(&denied));
    assert_eq!(denied.http(), None);
    let exception = proxy_exception(&denied);
    assert_eq!(exception.message, TRANSIENT_MESSAGE);
    assert_eq!(exception.error_type, "no_db_connection");
    assert_eq!(exception.code, "503");

    let text_deny = AllowRequestsOnOutage::Text("false".to_owned());
    let refused = get_org_object_for_request(&lookup, "o1", &text_deny)
        .await
        .unwrap_err();
    assert!(is_outage(&refused));

    let unsupported =
        get_org_object_for_request(&lookup, "o1", &AllowRequestsOnOutage::Unsupported)
            .await
            .unwrap_err();
    assert!(matches!(
        &unsupported,
        OrgLookupError::Failure(OrgFailure::Policy(_))
    ));
    assert!(!is_outage(&unsupported));
    assert_eq!(
        unsupported.to_string(),
        "general_settings.allow_requests_on_db_unavailable must be a boolean or a string"
    );
    assert_eq!(
        as_auth_failure(&unsupported),
        AuthFailure::Other {
            message: unsupported.to_string(),
            param: None,
        }
    );
}

#[tokio::test]
async fn last_known_local_no_refresh() {
    let world = world();
    world.seed_org(with_budget(org_row("o1", None, "b1"), "10.0"));
    let lookup = support::lookup(&world);
    let policy = AllowRequestsOnOutage::Boolean(false);
    let first = get_org_object_for_request(&lookup, "o1", &policy)
        .await
        .unwrap()
        .unwrap();
    assert!(first.litellm_budget_table.is_some());
    assert_events(
        &world,
        &[
            "read org_id:o1:with_budget local=false",
            "org o1 budget=true",
            "write org_id:o1:with_budget ttl=5",
            "read org_id:o1:with_budget:last_known local=true",
            "write org_id:o1:with_budget:last_known ttl=60",
        ],
    );
    world.clear_events();
    let second = get_org_object_for_request(&lookup, "o1", &policy)
        .await
        .unwrap()
        .unwrap();
    assert!(second.litellm_budget_table.is_some());
    assert_events(
        &world,
        &[
            "read org_id:o1:with_budget local=false",
            "read org_id:o1:with_budget:last_known local=true",
        ],
    );
}

#[tokio::test]
async fn last_known_write_failure() {
    let world = world();
    world.seed_org(with_budget(org_row("o1", None, "b1"), "10.0"));
    world.script("write org_id:o1:with_budget:last_known");
    let lookup = support::lookup(&world);
    let error = get_org_object_for_request(&lookup, "o1", &AllowRequestsOnOutage::Boolean(true))
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        OrgLookupError::Failure(OrgFailure::Cache(_))
    ));
    assert_eq!(error.http(), None);
    assert!(!is_outage(&error));
    assert!(world.cached_org("org_id:o1:with_budget").is_some());
}

#[tokio::test]
async fn project_no_db_before_cache() {
    let world = world();
    world.hold_project(
        "project_id:p1",
        cached_project(project_row("p1", Some("1.0"))),
    );
    let lookup = support::without_database(&world);
    let project = get_project_object(&lookup, "p1", 1.0_f64).await.unwrap();
    assert!(project.is_none());
    assert_events(&world, &[]);
}

#[tokio::test]
async fn project_includes_budget() {
    let world = world();
    world.seed_project(project_row("p1", Some("10.5")));
    let lookup = support::lookup(&world);
    let project = get_project_object(&lookup, "p1", 7.5_f64)
        .await
        .unwrap()
        .unwrap();
    let budget = project.project.litellm_budget_table.unwrap();
    assert_eq!(budget.budget_id.as_deref(), Some("b1"));
    assert!(budget.max_budget.is_some());
    assert_events(
        &world,
        &[
            "read project_id:p1",
            "project p1",
            "write project_id:p1 ttl=60",
        ],
    );
    assert!(world.cached_project("project_id:p1").is_some());
}

#[tokio::test]
async fn project_clock_and_cache() {
    let world = world();
    world.seed_project(project_row("p1", None));
    let lookup = support::lookup(&world);
    let first = get_project_object(&lookup, "p1", 1234.5_f64)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.last_refreshed_at, Some(1234.5_f64));
    assert_eq!(
        world
            .cached_project("project_id:p1")
            .map(|project| project.last_refreshed_at),
        Some(Some(1234.5_f64))
    );
    world.clear_events();
    let second = get_project_object(&lookup, "p1", 9999.0_f64)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        second.last_refreshed_at,
        Some(1234.5_f64),
        "a cache hit keeps the stored refresh time"
    );
    assert_events(&world, &["read project_id:p1"]);
}

#[tokio::test]
async fn project_failure_propagates() {
    let world = world();
    let lookup = support::lookup(&world);
    world.script("fault project");
    let fault = get_project_object(&lookup, "p1", 1.0_f64)
        .await
        .unwrap_err();
    assert!(matches!(&fault, OrgLookupError::Failure(_)));
    assert!(!is_outage(&fault));
    world.lock().failing.clear();
    world.script("outage project");
    world.clear_events();
    let outage = get_project_object(&lookup, "p1", 1.0_f64)
        .await
        .unwrap_err();
    assert!(is_outage(&outage));
    assert_eq!(proxy_exception(&outage).code, "503");
    assert_events(&world, &["read project_id:p1"]);
    assert!(world.cached_project("project_id:p1").is_none());
}

#[tokio::test]
async fn no_negative_cache() {
    let world = world();
    let lookup = support::lookup(&world);
    assert!(get_org_object(&lookup, org_id("o1"), true).await.is_err());
    assert!(
        get_project_object(&lookup, "p1", 1.0_f64)
            .await
            .unwrap()
            .is_none()
    );
    assert!(get_org_object_by_alias(&lookup, "gone").await.is_err());
    assert!(
        !world
            .events()
            .iter()
            .any(|event| event.starts_with("write "))
    );
    assert!(world.lock().orgs.is_empty());
    assert!(world.lock().projects.is_empty());
}

#[tokio::test]
async fn outage_message_reads_chain() {
    let policy = AllowRequestsOnOutage::Boolean(false);
    let permanent_world = world();
    permanent_world.script("permanent org");
    let permanent_lookup = support::lookup(&permanent_world);
    let permanent = get_org_object_for_request(&permanent_lookup, "o1", &policy)
        .await
        .unwrap_err();
    let AuthFailure::Proxy {
        exception: permanent_exception,
        ..
    } = as_auth_failure(&permanent)
    else {
        panic!("a permanent fault answers the 503 envelope");
    };
    assert_eq!(permanent_exception.code, "503");
    assert_eq!(permanent_exception.error_type, "no_db_connection");
    assert_eq!(permanent_exception.param.as_deref(), Some("None"));
    assert!(
        permanent_exception
            .message
            .contains("not a transient outage"),
        "a permanent fault names the engine fault: {}",
        permanent_exception.message
    );
    let transient_world = world();
    transient_world.script("outage org");
    let transient_lookup = support::lookup(&transient_world);
    let transient = get_org_object_for_request(&transient_lookup, "o1", &policy)
        .await
        .unwrap_err();
    let AuthFailure::Proxy {
        exception: transient_exception,
        ..
    } = as_auth_failure(&transient)
    else {
        panic!("a transport outage answers the 503 envelope");
    };
    assert_eq!(transient_exception.message, TRANSIENT_MESSAGE);
}
