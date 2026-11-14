#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::error::CatalogLookupError;
use super::lookup::CatalogResolver;
use super::scope::litellm_model;
use super::types::{Authority, CandidateView, Candidates, CatalogMatch, LookupRequest};
use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_identity::spec::DeploymentSpec;

// model_name, id, litellm model, team id and team public name (empty when the deployment has no team)
type Row = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

const BASE: &[Row] = &[
    ("gpt-4o", "a", "openai/gpt-4o", "", ""),
    ("gpt-4o", "b", "azure/gpt4o", "", ""),
    ("claude", "c", "anthropic/claude", "", ""),
];
const TEAM: &[Row] = &[("internal", "t1d", "openai/m", "t1", "team-gpt")];
const TEAM_AND_GLOBAL: &[Row] = &[
    ("team-gpt", "g", "openai/g", "", ""),
    ("internal", "t1d", "openai/m", "t1", "team-gpt"),
];
const TWO_TEAMS: &[Row] = &[
    ("one", "t1d", "openai/m", "t1", "shared"),
    ("two", "t2d", "openai/m", "t2", "shared"),
];
const WILDCARD: &[Row] = &[("gpt-*", "w", "openai/gpt-*", "", "")];
const MARKERS: &[Row] = &[
    ("g", "m", "auto_router/x", "", ""),
    ("g", "r", "openai/g", "", ""),
];

#[derive(Default)]
struct View {
    group: Option<Vec<DeploymentSpec>>,
    fallback: Option<String>,
    provider: Option<String>,
    reservation: Option<String>,
    no_access: Vec<String>,
    no_reserve: Vec<String>,
}

fn without(deployments: Vec<DeploymentSpec>, ids: &[String]) -> Vec<DeploymentSpec> {
    deployments
        .into_iter()
        .filter(|spec| !ids.iter().any(|id| Some(id.as_str()) == spec.id()))
        .collect()
}

impl CandidateView for View {
    fn routing_group(&self, _: &str, _: Option<&str>) -> Option<Vec<DeploymentSpec>> {
        self.group.clone()
    }

    fn filter_access_groups(
        &self,
        _: &str,
        deployments: Vec<DeploymentSpec>,
        _: Option<&str>,
    ) -> Vec<DeploymentSpec> {
        without(deployments, &self.no_access)
    }

    fn filter_reserved(
        &self,
        deployments: Vec<DeploymentSpec>,
        _: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
        if let Some(message) = &self.reservation {
            return Err(CatalogLookupError::Reserved {
                message: message.clone(),
            });
        }
        Ok(without(deployments, &self.no_reserve))
    }

    fn default_fallback(&self) -> Option<String> {
        self.fallback.clone()
    }

    fn inferred_provider(&self, _: &str) -> Option<String> {
        self.provider.clone()
    }
}

fn spec(name: &str, id: &str, model: &str, team: &str, public: &str) -> DeploymentSpec {
    let team_info = if team.is_empty() {
        String::new()
    } else {
        format!(r#","team_id":"{team}","team_public_model_name":"{public}""#)
    };
    let params: JsonObject = serde_json::from_str(&format!(r#"{{"model":"{model}"}}"#)).unwrap();
    let info: JsonObject = serde_json::from_str(&format!(r#"{{"id":"{id}"{team_info}}}"#)).unwrap();
    DeploymentSpec::new(
        name.to_owned(),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

fn pair(value: &str) -> (String, String) {
    let (from, to) = value.split_once('>').unwrap();
    (from.to_owned(), to.to_owned())
}

fn ids(value: &str) -> Vec<String> {
    value.split(',').map(str::to_owned).collect()
}

fn built(rows: &[Row]) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for (name, id, litellm, owner, public) in rows {
        let deployment = spec(name, id, litellm, owner, public);
        catalog.index_patterns(&deployment).unwrap();
        catalog.model_names.insert((*name).to_owned());
        catalog.push(deployment);
    }
    catalog
}

fn describe(found: &CatalogMatch) -> String {
    let kind = if matches!(found.candidates, Candidates::Single(_)) {
        "one"
    } else {
        "pool"
    };
    let members: Vec<String> = found
        .candidates
        .deployments()
        .iter()
        .map(|member| {
            format!(
                "{}@{}",
                member.id().unwrap_or_default(),
                litellm_model(member)
            )
        })
        .collect();
    let steps: Vec<String> = found
        .aliases
        .iter()
        .map(|step| format!("{:?}:{}>{}", step.source, step.from, step.to))
        .collect();
    format!(
        "{}|{kind}|{}|{}",
        found.group,
        members.join(","),
        steps.join(",")
    )
}

fn answer(
    resolver: &CatalogResolver<'_>,
    request: &LookupRequest<'_>,
    for_request: bool,
) -> String {
    if for_request {
        return match resolver.deployments_for_request(request) {
            Ok(found) => found
                .iter()
                .filter_map(DeploymentSpec::id)
                .collect::<Vec<_>>()
                .join(","),
            Err(error) => format!("{error:?}"),
        };
    }
    match resolver.resolve(request) {
        Ok(found) => describe(&found),
        Err(error) => format!("{error:?}"),
    }
}

// flags: team=id admin specific for_request ghost alias=a>b map=a>b fallback=m provider=p reservation=text group=ids no_access=ids no_reserve=ids
fn lookup(rows: &[Row], model: &str, team: Option<&str>, flags: &str) -> String {
    let mut catalog = built(rows);
    let (mut view, mut aliases, mut map) = (View::default(), BTreeMap::new(), BTreeMap::new());
    let mut request = LookupRequest {
        model,
        specific_deployment: false,
        authority: Authority {
            team_id: team,
            proxy_admin: false,
        },
    };
    let mut for_request = false;
    for token in flags.split_whitespace() {
        let (key, value) = token.split_once('=').unwrap_or((token, ""));
        match key {
            "admin" => request.authority.proxy_admin = true,
            "team" => request.authority.team_id = Some(value),
            "specific" => request.specific_deployment = true,
            "for_request" => for_request = true,
            "no_team_index" => catalog.team_index.clear(),
            "ghost" => catalog
                .index_patterns(&spec("ghost-*", "ghost", "openai/*", "", ""))
                .unwrap(),
            "alias" => aliases.extend([pair(value)]),
            "map" => map.extend([pair(value)]),
            "fallback" => view.fallback = Some(value.to_owned()),
            "provider" => view.provider = Some(value.to_owned()),
            "reservation" => view.reservation = Some(value.to_owned()),
            "group" => {
                let members = ids(value);
                let chosen = catalog
                    .models
                    .iter()
                    .filter(|member| members.iter().any(|id| Some(id.as_str()) == member.id()))
                    .cloned()
                    .collect();
                view.group = Some(chosen);
            }
            "no_access" => view.no_access = ids(value),
            "no_reserve" => view.no_reserve = ids(value),
            other => return format!("unknown flag {other}"),
        }
    }
    let resolver = CatalogResolver {
        catalog: &catalog,
        model_group_alias: &aliases,
        model_alias_map: &map,
        view: &view,
    };
    answer(&resolver, &request, for_request)
}

#[rstest]
#[case::plain_name_lists_every_deployment(
    BASE,
    "gpt-4o",
    None,
    "",
    "gpt-4o|pool|a@openai/gpt-4o,b@azure/gpt4o|"
)]
#[case::group_alias_rewrites_the_group(
    BASE,
    "smart",
    None,
    "alias=smart>gpt-4o",
    "gpt-4o|pool|a@openai/gpt-4o,b@azure/gpt4o|ModelGroupAlias:smart>gpt-4o"
)]
#[case::deployment_id_selects_one(
    BASE,
    "c",
    None,
    "",
    "anthropic/claude|one|c@anthropic/claude|DeploymentId:c>anthropic/claude"
)]
#[case::specific_deployment_matches_the_litellm_model(
    BASE,
    "azure/gpt4o",
    None,
    "specific",
    "azure/gpt4o|pool|b@azure/gpt4o|"
)]
#[case::litellm_model_is_the_last_lookup(
    BASE,
    "azure/gpt4o",
    None,
    "",
    "azure/gpt4o|pool|b@azure/gpt4o|"
)]
#[case::wildcard_fills_in_the_model(WILDCARD, "gpt-5", None, "", "gpt-5|pool|w@openai/gpt-5|")]
#[case::inferred_provider_prefixes_the_wildcard_lookup(&[("openai/*", "w", "openai/*", "", "")], "gpt-5", None, "provider=openai", "gpt-5|pool|w@openai/gpt-5|")]
#[case::team_public_name(TEAM, "team-gpt", Some("t1"), "", "team-gpt|pool|t1d@openai/m|")]
#[case::team_name_shadows_a_wildcard(&[TEAM[0], ("team-*", "w", "openai/*", "", "")], "team-gpt", Some("t1"), "", "team-gpt|pool|t1d@openai/m|")]
#[case::team_wildcard_serves_its_team(&[("internal", "t1d", "openai/m", "t1", "tm-*")], "tm-5", Some("t1"), "", "tm-5|pool|t1d@openai/m|")]
#[case::default_fallback_takes_over(
    BASE,
    "nope",
    None,
    "fallback=claude",
    "claude|pool|c@anthropic/claude|"
)]
#[case::model_alias_map_renames_the_group(
    BASE,
    "claude",
    None,
    "map=claude>claude-3",
    "claude-3|pool|c@anthropic/claude|ModelAliasMap:claude>claude-3"
)]
#[case::routing_group_members_are_used(BASE, "g", None, "group=b", "g|pool|b@azure/gpt4o|")]
#[case::admin_reaches_a_team_name(TEAM, "team-gpt", None, "admin", "team-gpt|pool|t1d@openai/m|")]
#[case::strategy_markers_are_dropped(MARKERS, "g", None, "", "g|pool|r@openai/g|")]
#[case::team_name_beats_a_global_one(
    TEAM_AND_GLOBAL,
    "team-gpt",
    Some("t1"),
    "",
    "team-gpt|pool|t1d@openai/m|"
)]
#[case::other_teams_get_the_global_one(
    TEAM_AND_GLOBAL,
    "team-gpt",
    Some("t2"),
    "",
    "team-gpt|pool|g@openai/g|"
)]
#[case::request_alias_lists_the_target(
    BASE,
    "smart",
    None,
    "for_request alias=smart>gpt-4o",
    "a,b"
)]
#[case::request_team_lists_its_deployments(TEAM, "team-gpt", Some("t1"), "for_request", "t1d")]
#[case::request_admin_reaches_every_team(TEAM, "team-gpt", None, "for_request admin", "t1d")]
#[case::request_without_authority_sees_nothing(TEAM, "team-gpt", None, "for_request", "")]
#[case::request_admin_keeps_the_name_match(BASE, "gpt-4o", None, "for_request admin", "a,b")]
#[case::team_deployment_by_its_internal_name(
    TEAM,
    "internal",
    None,
    "",
    "internal|pool|t1d@openai/m|"
)]
#[case::team_deployment_by_its_internal_name_for_its_team(
    TEAM,
    "internal",
    Some("t1"),
    "",
    "internal|pool|t1d@openai/m|"
)]
#[case::team_scan_finds_what_the_index_missed(
    TEAM,
    "team-gpt",
    Some("t1"),
    "no_team_index",
    "team-gpt|pool|t1d@openai/m|"
)]
fn deployment_catalog_behaviors(
    #[case] rows: &[Row],
    #[case] model: &str,
    #[case] team: Option<&str>,
    #[case] flags: &str,
    #[case] expected: &str,
) {
    assert_eq!(lookup(rows, model, team, flags), expected);
}

#[rstest]
#[case::unknown_model(BASE, "nope", "", r#"NoHealthyDeployments { model: "nope" }"#)]
#[case::only_markers(&MARKERS[..1], "g", "", r#"OnlyStrategyMarkers { model: "g" }"#)]
#[case::only_markers_by_litellm_model(&MARKERS[..1], "auto_router/x", "specific", r#"OnlyStrategyMarkers { model: "auto_router/x" }"#)]
#[case::two_teams_are_ambiguous(
    TWO_TEAMS,
    "shared",
    "admin",
    r#"AmbiguousTeams { model: "shared" }"#
)]
#[case::two_teams_are_ambiguous_for_a_request(
    TWO_TEAMS,
    "shared",
    "for_request admin",
    r#"AmbiguousTeams { model: "shared" }"#
)]
#[case::no_authority_no_team_names(
    TEAM,
    "team-gpt",
    "",
    r#"NoHealthyDeployments { model: "team-gpt" }"#
)]
#[case::other_team_cannot_see_it(
    TEAM,
    "team-gpt",
    "team=t2",
    r#"NoHealthyDeployments { model: "team-gpt" }"#
)]
#[case::empty_team_id_is_no_team(TEAM, "team-gpt", "team= admin", "team-gpt|pool|t1d@openai/m|")]
#[case::access_filter_blocks_the_fallback(
    BASE,
    "claude",
    "no_access=c fallback=gpt-4o",
    r#"NoHealthyDeployments { model: "claude" }"#
)]
#[case::access_filter_blocks_the_fallback_after_the_litellm_retry(
    BASE,
    "anthropic/claude",
    "no_access=c fallback=gpt-4o",
    r#"NoHealthyDeployments { model: "anthropic/claude" }"#
)]
#[case::missing_fallback_names_itself(
    BASE,
    "nope",
    "fallback=also-missing",
    r#"NoHealthyDeployments { model: "also-missing" }"#
)]
#[case::reservation_message_passes_through(
    BASE,
    "gpt-4o",
    "reservation=held",
    r#"Reserved { message: "held" }"#
)]
#[case::reserved_id_is_a_bad_request(
    BASE,
    "c",
    "no_reserve=c",
    r#"NoHealthyDeployments { model: "anthropic/claude" }"#
)]
#[case::unindexed_wildcard_candidate(
    BASE,
    "ghost-1",
    "ghost",
    r#"UnindexedCandidate { model_id: Some("ghost") }"#
)]
#[case::empty_alias_target_rewrites_nothing(
    BASE,
    "gpt",
    "alias=gpt>",
    r#"NoHealthyDeployments { model: "gpt" }"#
)]
#[case::wildcard_match_skips_the_alias_map(
    WILDCARD,
    "gpt-5",
    "map=gpt-5>zzz",
    "gpt-5|pool|w@openai/gpt-5|"
)]
#[case::registered_wildcard_name_is_a_plain_name(
    WILDCARD,
    "gpt-*",
    "",
    "gpt-*|pool|w@openai/gpt-*|"
)]
#[case::alias_map_applies_to_the_fallback(
    BASE,
    "nope",
    "fallback=claude map=claude>c3",
    "c3|pool|c@anthropic/claude|ModelAliasMap:claude>c3"
)]
#[case::request_team_caller_is_never_widened(TEAM, "team-gpt", "for_request admin team=t2", "")]
#[case::team_caller_unknown_name_finds_nothing(
    BASE,
    "nope",
    "team=t1",
    r#"NoHealthyDeployments { model: "nope" }"#
)]
#[case::team_deployment_is_hidden_from_other_teams(
    TEAM,
    "internal",
    "team=t2",
    r#"NoHealthyDeployments { model: "internal" }"#
)]
#[case::unnamed_deployment_is_not_found_by_name(
    &[("", "u", "openai/u", "", "")],
    "",
    "",
    r#"NoHealthyDeployments { model: "" }"#
)]
#[case::wildcard_needs_a_provider_for_the_prefixed_lookup(&[("openai/*", "w", "openai/*", "", "")], "gpt-5", "", r#"NoHealthyDeployments { model: "gpt-5" }"#)]
#[case::team_wildcard_needs_its_team(&[("internal", "t1d", "openai/m", "t1", "tm-*")], "tm-5", "", r#"NoHealthyDeployments { model: "tm-5" }"#)]
fn deployment_catalog_boundaries(
    #[case] catalog: &[Row],
    #[case] requested: &str,
    #[case] options: &str,
    #[case] expected: &str,
) {
    // a lookup reads one snapshot and changes nothing, so asking twice answers the same
    let outcomes = [(); 2].map(|()| lookup(catalog, requested, None, options));
    assert!(
        outcomes.iter().all(|outcome| outcome == expected),
        "{requested} with {options:?} gave {outcomes:?}"
    );
}
