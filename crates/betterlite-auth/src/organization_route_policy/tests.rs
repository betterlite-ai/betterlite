#![cfg(test)]

use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use serde_json::Number;

use super::error::RoutePolicyError;
use super::membership::{ORG_ADMIN, OrganizationMembership, PROXY_ADMIN, RouteUser};
use super::org_admin::user_is_org_admin;
use super::role_check::{ORGANIZATION_CREATE_ROUTE, TEAM_CREATE_ROUTE, check_organization_role};
use super::team_context::{
    PATCH_TEAM_ROUTE_TEMPLATE, TeamOrganizationLookup, add_team_org_context,
};

const INFO_ROUTE: &str = "/organization/info";
const TEAM_UPDATE_ROUTE: &str = "/team/update";

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn membership(organization_id: Option<&str>, user_role: Option<&str>) -> OrganizationMembership {
    OrganizationMembership {
        organization_id: organization_id.map(str::to_owned),
        user_role: user_role.map(str::to_owned),
    }
}

fn user(user_role: Option<&str>, memberships: Option<Vec<OrganizationMembership>>) -> RouteUser {
    RouteUser {
        user_role: user_role.map(str::to_owned),
        organization_memberships: memberships,
    }
}

fn org_admin(organization_id: &str) -> OrganizationMembership {
    membership(Some(organization_id), Some(ORG_ADMIN))
}

fn body(entries: &[(&str, Json)]) -> JsonObject {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn organization_body(organization_id: &str) -> JsonObject {
    body(&[("organization_id", text(organization_id))])
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
#[error("the scripted team lookup failed for {team_id}")]
struct LookupFailure {
    team_id: String,
}

struct ScriptedLookup {
    answer: Option<String>,
    failure: bool,
    fetches: Mutex<Vec<String>>,
}

impl ScriptedLookup {
    fn answering(organization_id: Option<&str>) -> Self {
        Self {
            answer: organization_id.map(str::to_owned),
            failure: false,
            fetches: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            answer: None,
            failure: true,
            fetches: Mutex::new(Vec::new()),
        }
    }

    fn fetches(&self) -> Vec<String> {
        self.fetches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl TeamOrganizationLookup for ScriptedLookup {
    type Failure = LookupFailure;

    async fn team_organization_id(&self, team_id: &str) -> Result<Option<String>, Self::Failure> {
        self.fetches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(team_id.to_owned());
        if self.failure {
            return Err(LookupFailure {
                team_id: team_id.to_owned(),
            });
        }
        Ok(self.answer.clone())
    }
}

#[test]
fn no_user_skips_policy() {
    let request = organization_body("org-a");
    assert_eq!(
        check_organization_role(None, ORGANIZATION_CREATE_ROUTE, &request),
        Ok(())
    );
    assert_eq!(check_organization_role(None, INFO_ROUTE, &request), Ok(()));
    assert_eq!(
        check_organization_role(None, TEAM_CREATE_ROUTE, &JsonObject::new()),
        Ok(())
    );
}

#[test]
fn organization_create_proxy_admin_only() {
    let request = JsonObject::new();
    let member = user(Some(ORG_ADMIN), Some(vec![org_admin("org-a")]));
    let denied = check_organization_role(Some(&member), ORGANIZATION_CREATE_ROUTE, &request);
    assert_eq!(
        denied,
        Err(RoutePolicyError::OrganizationCreateDenied {
            role: ORG_ADMIN.to_owned(),
        })
    );
    let exception = denied.unwrap_err().into_proxy_exception();
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param, Some("user_role".to_owned()));
    assert_eq!(exception.code, "401");
    assert!(
        exception
            .message
            .contains("Only proxy admins can create new organizations. You are org_admin")
    );
    // a caller with no role at all is named as None, and a proxy admin creates the organization
    let roleless = user(None, None);
    assert_eq!(
        check_organization_role(Some(&roleless), ORGANIZATION_CREATE_ROUTE, &request),
        Err(RoutePolicyError::OrganizationCreateDenied {
            role: "None".to_owned(),
        })
    );
    let admin = user(Some(PROXY_ADMIN), None);
    assert_eq!(
        check_organization_role(Some(&admin), ORGANIZATION_CREATE_ROUTE, &request),
        Ok(())
    );
    // no other route judges the caller by user_role
    assert_eq!(
        check_organization_role(Some(&member), TEAM_UPDATE_ROUTE, &request),
        Ok(())
    );
}

#[test]
fn proxy_admin_bypasses() {
    let admin = user(
        Some(PROXY_ADMIN),
        Some(vec![membership(Some("org-a"), Some("internal_user"))]),
    );
    assert_eq!(
        check_organization_role(Some(&admin), INFO_ROUTE, &JsonObject::new()),
        Ok(())
    );
    assert_eq!(
        check_organization_role(Some(&admin), INFO_ROUTE, &organization_body("org-b")),
        Ok(())
    );
    let no_memberships = user(Some(PROXY_ADMIN), None);
    assert_eq!(
        check_organization_role(Some(&no_memberships), INFO_ROUTE, &JsonObject::new()),
        Ok(())
    );
    let team_admin = user(Some(PROXY_ADMIN), Some(vec![org_admin("org-a")]));
    assert_eq!(
        check_organization_role(Some(&team_admin), TEAM_CREATE_ROUTE, &JsonObject::new()),
        Ok(())
    );
}

#[rstest]
#[case::organization_info("/organization/info")]
#[case::organization_delete("/organization/delete")]
#[case::member_add("/organization/member_add")]
#[case::member_update("/organization/member_update")]
#[case::member_delete("/organization/member_delete")]
fn org_admin_route_matrix(#[case] route: &str) {
    let member = user(None, Some(vec![org_admin("org-a")]));
    assert_eq!(
        check_organization_role(Some(&member), route, &organization_body("org-a")),
        Ok(()),
    );
    assert_eq!(
        check_organization_role(Some(&member), route, &organization_body("org-b")),
        Err(RoutePolicyError::NoRoleInOrganization {
            organization_id: "org-b".to_owned(),
        }),
    );
}

#[test]
fn memberships_null_vs_empty() {
    let no_memberships = user(None, None);
    let not_member = check_organization_role(
        Some(&no_memberships),
        INFO_ROUTE,
        &organization_body("org-a"),
    );
    assert_eq!(
        not_member,
        Err(RoutePolicyError::NotAMember {
            route: INFO_ROUTE.to_owned(),
        })
    );
    let exception = not_member.unwrap_err().into_proxy_exception();
    assert_eq!(exception.param, Some("organization_id".to_owned()));
    assert_eq!(exception.code, "401");
    assert!(exception.message.contains("Tried to access route=/organization/info but you are not a member of any organization. Please contact the proxy admin to request access."));
    // an empty list is a member of no organization, which reads as no role for the selected one
    let empty = user(None, Some(Vec::new()));
    assert_eq!(
        check_organization_role(Some(&empty), INFO_ROUTE, &organization_body("org-a")),
        Err(RoutePolicyError::NoRoleInOrganization {
            organization_id: "org-a".to_owned(),
        })
    );
    assert_eq!(
        check_organization_role(Some(&empty), INFO_ROUTE, &JsonObject::new()),
        Err(RoutePolicyError::MissingOrganizationId)
    );
}

#[test]
fn missing_selected_org() {
    let member = user(None, Some(vec![org_admin("org-a")]));
    assert_eq!(
        check_organization_role(Some(&member), INFO_ROUTE, &JsonObject::new()),
        Err(RoutePolicyError::MissingOrganizationId)
    );
    let null = body(&[("organization_id", Json::Null)]);
    assert_eq!(
        check_organization_role(Some(&member), INFO_ROUTE, &null),
        Err(RoutePolicyError::MissingOrganizationId)
    );
    // a number is not the declared text, so the request boundary names the field
    let numbered = body(&[("organization_id", number(7))]);
    let not_text = check_organization_role(Some(&member), INFO_ROUTE, &numbered);
    assert_eq!(not_text, Err(RoutePolicyError::OrganizationIdNotText));
    let exception = not_text.unwrap_err().into_proxy_exception();
    assert_eq!(exception.error_type, "invalid_request_error");
    assert_eq!(exception.param, Some("organization_id".to_owned()));
    assert_eq!(exception.code, "400");
}

#[test]
fn missing_org_role() {
    let member = user(None, Some(vec![org_admin("org-a")]));
    let denied = check_organization_role(Some(&member), INFO_ROUTE, &organization_body("org-b"));
    assert_eq!(
        denied,
        Err(RoutePolicyError::NoRoleInOrganization {
            organization_id: "org-b".to_owned(),
        })
    );
    let exception = denied.unwrap_err().into_proxy_exception();
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param, Some("organization_id".to_owned()));
    assert_eq!(exception.code, "401");
    assert!(exception.message.contains("You do not have a role within the selected organization. Passed organization_id: org-b. Please contact the organization admin to request access."));
    // a membership that stored no role reads as no role, because mapping.get answers None for it too
    let roleless = user(None, Some(vec![membership(Some("org-b"), None)]));
    assert_eq!(
        check_organization_role(Some(&roleless), INFO_ROUTE, &organization_body("org-b")),
        Err(RoutePolicyError::NoRoleInOrganization {
            organization_id: "org-b".to_owned(),
        })
    );
}

#[test]
fn wrong_role_error_param() {
    let member = user(
        None,
        Some(vec![membership(Some("org-a"), Some("internal_user"))]),
    );
    let denied = check_organization_role(Some(&member), INFO_ROUTE, &organization_body("org-a"));
    assert_eq!(
        denied,
        Err(RoutePolicyError::NotOrganizationAdmin {
            route: INFO_ROUTE.to_owned(),
            organization_id: "org-a".to_owned(),
            role: "internal_user".to_owned(),
        })
    );
    let exception = denied.unwrap_err().into_proxy_exception();
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param, Some("user_role".to_owned()));
    assert_eq!(exception.code, "401");
    assert!(exception.message.contains("You do not have the required role to perform /organization/info in Organization org-a. Your role is internal_user in Organization org-a"));
}

#[test]
fn one_membership_requires_org_on_team_create() {
    let member = user(None, Some(vec![org_admin("org-a")]));
    let missing = check_organization_role(Some(&member), TEAM_CREATE_ROUTE, &JsonObject::new());
    assert_eq!(
        missing,
        Err(RoutePolicyError::TeamCreateNeedsOrganization {
            organizations: "org-a".to_owned(),
        })
    );
    let exception = missing.unwrap_err().into_proxy_exception();
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param, Some("organization_id".to_owned()));
    assert!(
        exception
            .message
            .contains("You are part of multiple organizations: org-a")
    );
    assert_eq!(
        check_organization_role(
            Some(&member),
            TEAM_CREATE_ROUTE,
            &organization_body("org-a")
        ),
        Ok(())
    );
}

#[test]
fn team_create_without_memberships() {
    let no_memberships = user(None, None);
    assert_eq!(
        check_organization_role(Some(&no_memberships), TEAM_CREATE_ROUTE, &JsonObject::new()),
        Ok(())
    );
    let empty = user(Some("internal_user"), Some(Vec::new()));
    assert_eq!(
        check_organization_role(Some(&empty), TEAM_CREATE_ROUTE, &JsonObject::new()),
        Ok(())
    );
    // a selection outside the memberships has no role, so the param names user_role
    let elsewhere = user(None, Some(vec![org_admin("org-a")]));
    let wrong = check_organization_role(
        Some(&elsewhere),
        TEAM_CREATE_ROUTE,
        &organization_body("org-b"),
    );
    assert_eq!(
        wrong,
        Err(RoutePolicyError::TeamCreateNeedsOrgAdmin {
            route: TEAM_CREATE_ROUTE.to_owned(),
            organization_id: "org-b".to_owned(),
            role: "None".to_owned(),
        })
    );
    let exception = wrong.unwrap_err().into_proxy_exception();
    assert_eq!(exception.param, Some("user_role".to_owned()));
    // an explicit empty selection is present, so it reaches the role check with an id no membership carries
    assert_eq!(
        check_organization_role(Some(&elsewhere), TEAM_CREATE_ROUTE, &organization_body("")),
        Err(RoutePolicyError::TeamCreateNeedsOrgAdmin {
            route: TEAM_CREATE_ROUTE.to_owned(),
            organization_id: String::new(),
            role: "None".to_owned(),
        })
    );
}

#[test]
fn duplicate_memberships_last_role_wins() {
    let demoted = user(
        None,
        Some(vec![
            org_admin("org-a"),
            membership(Some("org-a"), Some("internal_user")),
        ]),
    );
    let info = demoted.organization_info();
    assert_eq!(
        info.organizations,
        vec!["org-a".to_owned(), "org-a".to_owned()]
    );
    assert_eq!(
        info.roles.get("org-a"),
        Some(&Some("internal_user".to_owned()))
    );
    assert_eq!(
        check_organization_role(Some(&demoted), INFO_ROUTE, &organization_body("org-a")),
        Err(RoutePolicyError::NotOrganizationAdmin {
            route: INFO_ROUTE.to_owned(),
            organization_id: "org-a".to_owned(),
            role: "internal_user".to_owned(),
        })
    );
    // the last membership is the one that decides the role
    let promoted = user(
        None,
        Some(vec![
            membership(Some("org-a"), Some("internal_user")),
            org_admin("org-a"),
        ]),
    );
    assert_eq!(
        promoted.organization_info().roles.get("org-a"),
        Some(&Some(ORG_ADMIN.to_owned()))
    );
    assert_eq!(
        check_organization_role(Some(&promoted), INFO_ROUTE, &organization_body("org-a")),
        Ok(())
    );
}

#[test]
fn null_membership_ids_ignored() {
    let mixed = user(
        None,
        Some(vec![membership(None, Some(ORG_ADMIN)), org_admin("org-a")]),
    );
    let info = mixed.organization_info();
    assert_eq!(info.organizations, vec!["org-a".to_owned()]);
    assert_eq!(info.roles.len(), 1);
    assert_eq!(
        check_organization_role(Some(&mixed), INFO_ROUTE, &organization_body("org-a")),
        Ok(())
    );
    // the null row is no organization, so no selection finds a role through it
    let null_only = user(None, Some(vec![membership(None, Some(ORG_ADMIN))]));
    assert_eq!(
        check_organization_role(Some(&null_only), INFO_ROUTE, &organization_body("")),
        Err(RoutePolicyError::NoRoleInOrganization {
            organization_id: String::new(),
        })
    );
    // /team/new counts the row as a membership, so it still demands the organization_id
    assert_eq!(
        check_organization_role(Some(&null_only), TEAM_CREATE_ROUTE, &JsonObject::new()),
        Err(RoutePolicyError::TeamCreateNeedsOrganization {
            organizations: String::new(),
        })
    );
}

#[test]
fn all_candidate_organizations() {
    let both = user(None, Some(vec![org_admin("org-a"), org_admin("org-b")]));
    let one = user(None, Some(vec![org_admin("org-a")]));
    assert!(user_is_org_admin(&organization_body("org-a"), Some(&both)));
    assert!(!user_is_org_admin(&organization_body("org-c"), Some(&both)));
    assert!(user_is_org_admin(
        &body(&[("organizations", Json::List(vec![text("org-a")]))]),
        Some(&one)
    ));
    assert!(user_is_org_admin(
        &body(&[(
            "organizations",
            Json::List(vec![text("org-a"), text("org-b")])
        )]),
        Some(&both)
    ));
    assert!(!user_is_org_admin(
        &body(&[(
            "organizations",
            Json::List(vec![text("org-a"), text("org-c")])
        )]),
        Some(&one)
    ));
    // the singular and the list arrive together, and every candidate must be administered
    let requested = body(&[
        ("organization_id", text("org-a")),
        (
            "organizations",
            Json::List(vec![text("org-a"), text("org-b")]),
        ),
    ]);
    assert!(user_is_org_admin(&requested, Some(&both)));
    assert!(!user_is_org_admin(&requested, Some(&one)));
    // duplicate candidates are harmless
    assert!(user_is_org_admin(
        &body(&[(
            "organizations",
            Json::List(vec![text("org-a"), text("org-a")])
        )]),
        Some(&one)
    ));
}

#[test]
fn no_candidates_and_nonlist_organizations() {
    let admin = user(None, Some(vec![org_admin("org-a")]));
    assert!(!user_is_org_admin(&JsonObject::new(), Some(&admin)));
    assert!(!user_is_org_admin(
        &body(&[("organizations", Json::List(Vec::new()))]),
        Some(&admin)
    ));
    // organizations counts only as a list, so a string naming one is no candidate
    assert!(!user_is_org_admin(
        &body(&[("organizations", text("org-a"))]),
        Some(&admin)
    ));
    assert!(!user_is_org_admin(
        &body(&[
            ("organization_id", Json::Null),
            ("organizations", Json::Null)
        ]),
        Some(&admin)
    ));
    // a candidate that is not text cannot be an organization the caller administers
    assert!(!user_is_org_admin(
        &body(&[("organization_id", Json::List(vec![text("org-a")]))]),
        Some(&admin)
    ));
    assert!(user_is_org_admin(
        &body(&[("organization_id", text("org-a"))]),
        Some(&admin)
    ));
}

#[test]
fn org_predicate_has_no_admin_shortcut() {
    let proxy = user(
        Some(PROXY_ADMIN),
        Some(vec![membership(Some("org-a"), Some("internal_user"))]),
    );
    assert!(!user_is_org_admin(
        &organization_body("org-a"),
        Some(&proxy)
    ));
    let empty_proxy = user(Some(PROXY_ADMIN), None);
    assert!(!user_is_org_admin(
        &organization_body("org-a"),
        Some(&empty_proxy)
    ));
    assert!(!user_is_org_admin(&organization_body("org-a"), None));
    // the organization membership is the only thing that answers true
    let admin = user(Some(PROXY_ADMIN), Some(vec![org_admin("org-a")]));
    assert!(user_is_org_admin(&organization_body("org-a"), Some(&admin)));
}

#[tokio::test]
async fn truthy_caller_org_retained() {
    let lookup = ScriptedLookup::answering(Some("org-team"));
    let request = body(&[
        ("organization_id", text("org-client")),
        ("team_id", text("team-1")),
        ("metadata", Json::Object(JsonObject::new())),
    ]);
    assert_eq!(
        add_team_org_context(TEAM_UPDATE_ROUTE, &request, &lookup, None)
            .await
            .unwrap(),
        request
    );
    assert!(lookup.fetches().is_empty());
    // the same holds on the template route, where the team id would otherwise come from the path
    let templated = body(&[("organization_id", text("org-client"))]);
    assert_eq!(
        add_team_org_context(
            "/team/team-9",
            &templated,
            &lookup,
            Some(PATCH_TEAM_ROUTE_TEMPLATE)
        )
        .await
        .unwrap(),
        templated
    );
    assert!(lookup.fetches().is_empty());
}

#[tokio::test]
async fn null_and_empty_org_enriched() {
    let lookup = ScriptedLookup::answering(Some("org-team"));
    let null = body(&[("organization_id", Json::Null), ("team_id", text("team-1"))]);
    let from_null = add_team_org_context(TEAM_UPDATE_ROUTE, &null, &lookup, None)
        .await
        .unwrap();
    assert_eq!(from_null.get("organization_id"), Some(&text("org-team")));
    assert_eq!(null.get("organization_id"), Some(&Json::Null));
    let empty = body(&[("organization_id", text("")), ("team_id", text("team-1"))]);
    let from_empty = add_team_org_context(TEAM_UPDATE_ROUTE, &empty, &lookup, None)
        .await
        .unwrap();
    assert_eq!(from_empty.get("organization_id"), Some(&text("org-team")));
    assert_eq!(
        lookup.fetches(),
        vec!["team-1".to_owned(), "team-1".to_owned()]
    );
    // the other fields survive the copy
    assert_eq!(from_empty.get("team_id"), empty.get("team_id"));
}

#[tokio::test]
async fn exact_patch_template_only() {
    let lookup = ScriptedLookup::answering(Some("org-team"));
    let empty = JsonObject::new();
    let from_template = add_team_org_context(
        "/team/team-9",
        &empty,
        &lookup,
        Some(PATCH_TEAM_ROUTE_TEMPLATE),
    )
    .await
    .unwrap();
    assert_eq!(
        from_template.get("organization_id"),
        Some(&text("org-team"))
    );
    assert_eq!(lookup.fetches(), vec!["team-9".to_owned()]);
    // a sibling /team/<verb> route without the exact template never looks a team up
    assert_eq!(
        add_team_org_context("/team/team-9", &empty, &lookup, None)
            .await
            .unwrap(),
        empty
    );
    assert_eq!(
        add_team_org_context("/team/team-9", &empty, &lookup, Some("/team/{team_alias}"))
            .await
            .unwrap(),
        empty
    );
    // /team/update reads the body's team id, not the last path segment
    let update = body(&[("team_id", text("team-7"))]);
    let from_update = add_team_org_context(
        TEAM_UPDATE_ROUTE,
        &update,
        &lookup,
        Some(PATCH_TEAM_ROUTE_TEMPLATE),
    )
    .await
    .unwrap();
    assert_eq!(from_update.get("organization_id"), Some(&text("org-team")));
    assert_eq!(
        lookup.fetches(),
        vec!["team-9".to_owned(), "team-7".to_owned()]
    );
    // the trailing slash leaves no last segment
    assert_eq!(
        add_team_org_context("/team/", &empty, &lookup, Some(PATCH_TEAM_ROUTE_TEMPLATE))
            .await
            .unwrap(),
        empty
    );
}

#[rstest]
#[case::number(Some(number(7)))]
#[case::boolean(Some(Json::Bool(true)))]
#[case::null(Some(Json::Null))]
#[case::object(Some(Json::Object(JsonObject::new())))]
#[case::empty(Some(text("")))]
#[case::missing(None)]
#[tokio::test]
async fn invalid_team_id_skips_lookup(#[case] team_id: Option<Json>) {
    let lookup = ScriptedLookup::answering(Some("org-team"));
    let request = match team_id {
        Some(value) => body(&[("team_id", value)]),
        None => JsonObject::new(),
    };
    assert_eq!(
        add_team_org_context(TEAM_UPDATE_ROUTE, &request, &lookup, None)
            .await
            .unwrap(),
        request
    );
    assert!(lookup.fetches().is_empty());
    // the control: a text team id is the one that reaches the lookup
    let text_id = body(&[("team_id", text("team-3"))]);
    let enriched = add_team_org_context(TEAM_UPDATE_ROUTE, &text_id, &lookup, None)
        .await
        .unwrap();
    assert_eq!(enriched.get("organization_id"), Some(&text("org-team")));
    assert_eq!(lookup.fetches(), vec!["team-3".to_owned()]);
}

#[tokio::test]
async fn missing_team_org_preserves_identity() {
    let request = body(&[
        ("team_id", text("team-1")),
        ("metadata", Json::Object(JsonObject::new())),
    ]);
    let absent = ScriptedLookup::answering(None);
    assert_eq!(
        add_team_org_context(TEAM_UPDATE_ROUTE, &request, &absent, None)
            .await
            .unwrap(),
        request
    );
    assert_eq!(absent.fetches(), vec!["team-1".to_owned()]);
    // an empty organization id is no context either, so the caller's body is the answer
    let empty = ScriptedLookup::answering(Some(""));
    assert_eq!(
        add_team_org_context(TEAM_UPDATE_ROUTE, &request, &empty, None)
            .await
            .unwrap(),
        request
    );
    assert_eq!(empty.fetches(), vec!["team-1".to_owned()]);
}

#[tokio::test]
async fn lookup_error_propagates() {
    let lookup = ScriptedLookup::failing();
    let request = body(&[("team_id", text("team-1"))]);
    let error = add_team_org_context(TEAM_UPDATE_ROUTE, &request, &lookup, None)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        LookupFailure {
            team_id: "team-1".to_owned(),
        }
    );
    assert_eq!(lookup.fetches(), vec!["team-1".to_owned()]);
}

#[tokio::test]
async fn enrichment_copies_body() {
    let lookup = ScriptedLookup::answering(Some("org-team"));
    let mut nested = JsonObject::new();
    nested.insert("budget".to_owned(), text("5"));
    let request = body(&[
        ("team_id", text("team-1")),
        ("team_alias", text("Alpha")),
        ("metadata", Json::Object(nested)),
    ]);
    let enriched = add_team_org_context(TEAM_UPDATE_ROUTE, &request, &lookup, None)
        .await
        .unwrap();
    assert_eq!(enriched.get("organization_id"), Some(&text("org-team")));
    assert_eq!(enriched.get("team_alias"), request.get("team_alias"));
    assert_eq!(enriched.get("metadata"), request.get("metadata"));
    // the caller's body keeps its fields and gains no organization_id
    assert_eq!(request.get("organization_id"), None);
    assert_eq!(request.len(), 3);
    assert_eq!(enriched.len(), 4);
}
