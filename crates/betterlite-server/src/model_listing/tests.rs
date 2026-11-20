#![cfg(test)]

mod available;
mod errors;
mod info;
mod response;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_model_lists::access_groups::AccessGroupScope;
use betterlite_auth::auth_model_lists::complete_list::ListingFlags;
use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_auth::auth_model_lists::fallbacks::{FallbackError, FallbackGroups};
use betterlite_auth::auth_model_lists::parameters::ModelParameters;
use betterlite_auth::auth_model_lists::providers::ProviderModels;
use betterlite_auth::auth_model_lists::wildcard::{DeploymentLookup, WildcardError};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::fallback_plans::Settings;
use betterlite_config::sections::fallback_plans::chains::{FallbackChainEntry, FallbackTarget};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use serde_json::Number;

use crate::model_listing::access::{
    AccessLookups, DeploymentListingInfo, ListingRouter, ListingSources, NotAMember, TeamListing,
    TeamMissing,
};
use crate::model_listing::available::AvailableModelsRequest;
use crate::model_listing::info::{
    ModelInfoLookupError, ModelInfoRequest, ModelInfoSource, ModelInfoSources,
};

const CREATED_AT: i64 = 1_677_610_602;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn key(fields: &str) -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(fields).expect("the key fixture parses");
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key fixture constructs")
}

fn team(models: &[&str]) -> TeamListing {
    TeamListing {
        team_id: "t1".to_owned(),
        models: strings(models),
        access_group_ids: Vec::new(),
    }
}

struct NoProviders;

impl ProviderModels for NoProviders {
    fn valid_models(
        &self,
        _provider: Option<&str>,
        _parameters: Option<&ModelParameters>,
    ) -> Option<Vec<String>> {
        None
    }
}

struct StaticProviders(Vec<String>);

impl ProviderModels for StaticProviders {
    fn valid_models(
        &self,
        provider: Option<&str>,
        _parameters: Option<&ModelParameters>,
    ) -> Option<Vec<String>> {
        match provider {
            None => Some(self.0.clone()),
            Some(_) => None,
        }
    }
}

#[derive(Default)]
struct FakeRouter {
    model_names: Vec<String>,
    access_groups: IndexMap<String, Vec<String>>,
    aliases: BTreeMap<String, String>,
    listings: BTreeMap<String, DeploymentListingInfo>,
    modes: BTreeMap<String, String>,
    rows_fail: bool,
    seen_teams: Mutex<Vec<Option<String>>>,
}

impl DeploymentLookup for FakeRouter {
    fn model_rows(
        &self,
        _model: &str,
        team_id: Option<&str>,
    ) -> Result<Vec<ModelParameters>, WildcardError> {
        self.seen_teams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(team_id.map(str::to_owned));
        if self.rows_fail {
            return Err(WildcardError::Lookup);
        }
        Ok(Vec::new())
    }
}

impl ListingRouter for FakeRouter {
    fn model_names(&self) -> Vec<String> {
        self.model_names.clone()
    }

    fn model_access_groups(&self) -> IndexMap<String, Vec<String>> {
        self.access_groups.clone()
    }

    fn model_group_alias(&self, model: &str) -> Option<String> {
        self.aliases.get(model).cloned()
    }

    fn listing_info(&self, model: &str) -> Option<DeploymentListingInfo> {
        self.listings.get(model).cloned()
    }

    fn configured_mode(&self, model: &str) -> Option<String> {
        self.modes.get(model).cloned()
    }
}

impl FakeRouter {
    #[must_use]
    fn with_model_names(mut self, names: &[&str]) -> Self {
        self.model_names = strings(names);
        self
    }

    #[must_use]
    fn with_failing_rows(mut self) -> Self {
        self.rows_fail = true;
        self
    }
}

#[derive(Default)]
struct FakeAccess {
    team_row: Option<TeamListing>,
    team_missing: bool,
    member: bool,
    team_group_models: Vec<String>,
    key_group_models: Vec<String>,
    ceiling: Option<BTreeSet<String>>,
    team_reads: AtomicUsize,
    validated: AtomicUsize,
}

#[async_trait]
impl AccessLookups for FakeAccess {
    async fn team(&self, team_id: &str) -> Result<Option<TeamListing>, TeamMissing> {
        self.team_reads.fetch_add(1, Ordering::SeqCst);
        if self.team_missing {
            return Err(TeamMissing {
                team_id: team_id.to_owned(),
            });
        }
        Ok(self.team_row.clone())
    }

    async fn validate_membership(
        &self,
        _key: &UserAPIKeyAuth,
        team: &TeamListing,
    ) -> Result<(), NotAMember> {
        self.validated.fetch_add(1, Ordering::SeqCst);
        if self.member {
            return Ok(());
        }
        Err(NotAMember {
            team_id: team.team_id.clone(),
        })
    }

    async fn team_access_group_models(&self, team: Option<&TeamListing>) -> Vec<String> {
        match team {
            Some(_) => self.team_group_models.clone(),
            None => Vec::new(),
        }
    }

    async fn key_access_group_models(
        &self,
        _key: &UserAPIKeyAuth,
        _team: Option<&TeamListing>,
    ) -> Vec<String> {
        self.key_group_models.clone()
    }

    async fn agent_ceiling(&self, _agent_id: &str) -> Option<BTreeSet<String>> {
        self.ceiling.clone()
    }
}

impl FakeAccess {
    #[must_use]
    fn with_member(mut self, member: bool) -> Self {
        self.member = member;
        self
    }

    #[must_use]
    fn with_team(mut self, team: TeamListing) -> Self {
        self.team_row = Some(team);
        self
    }

    #[must_use]
    fn with_missing_team(mut self) -> Self {
        self.team_missing = true;
        self
    }

    #[must_use]
    fn with_team_group_models(mut self, models: &[&str]) -> Self {
        self.team_group_models = strings(models);
        self
    }

    #[must_use]
    fn with_key_group_models(mut self, models: &[&str]) -> Self {
        self.key_group_models = strings(models);
        self
    }

    #[must_use]
    fn with_ceiling(mut self, ceiling: BTreeSet<String>) -> Self {
        self.ceiling = Some(ceiling);
        self
    }
}

#[derive(Default)]
struct FakeCostMap {
    entries: BTreeMap<String, JsonObject>,
    generalized: BTreeSet<String>,
    failing: BTreeSet<String>,
}

impl ModelInfoSource for FakeCostMap {
    fn model_info(&self, model: &str) -> Result<Option<JsonObject>, ModelInfoLookupError> {
        if self.failing.contains(model) {
            return Err(ModelInfoLookupError {
                model: model.to_owned(),
            });
        }
        Ok(self.entries.get(model).cloned())
    }

    fn is_generalized(&self, info: &JsonObject) -> bool {
        match info.get("key") {
            Some(Json::Text(key)) => self.generalized.contains(key),
            _ => false,
        }
    }
}

#[derive(Default)]
struct ExpectedGroups {
    chain: Vec<FallbackChainEntry>,
    answer: Vec<String>,
}

impl FallbackGroups for ExpectedGroups {
    fn fallback_group(
        &self,
        chain: &[FallbackChainEntry],
        _model: &str,
    ) -> Result<Option<Vec<String>>, FallbackError> {
        if chain == self.chain.as_slice() {
            return Ok(Some(self.answer.clone()));
        }
        Ok(None)
    }
}

const fn listing_flags() -> ListingFlags {
    ListingFlags {
        infer_model_from_keys: false,
        return_wildcard_routes: false,
        groups: AccessGroupScope {
            include: false,
            only: false,
        },
    }
}

const fn listing_request(team_id: Option<&str>, flags: ListingFlags) -> AvailableModelsRequest<'_> {
    AvailableModelsRequest {
        user_model: None,
        team_id,
        flags,
    }
}

fn listing_sources<'listing>(
    router: Option<&'listing FakeRouter>,
    access: &'listing FakeAccess,
    providers: &'listing dyn ProviderModels,
    credentials: &'listing CredentialSnapshot,
) -> ListingSources<'listing> {
    ListingSources {
        router: router.map(|fake: &FakeRouter| -> &dyn ListingRouter { fake }),
        access,
        providers,
        credentials,
    }
}

fn info_sources<'info>(
    router: Option<&'info FakeRouter>,
    cost_map: &'info FakeCostMap,
    settings: Option<&'info Settings>,
    groups: &'info ExpectedGroups,
) -> ModelInfoSources<'info> {
    ModelInfoSources {
        router: router.map(|fake: &FakeRouter| -> &dyn ListingRouter { fake }),
        cost_map,
        fallback_settings: settings,
        fallback_groups: groups,
        created_at: CREATED_AT,
    }
}

const fn info_request(model_id: &str) -> ModelInfoRequest<'_> {
    ModelInfoRequest {
        model_id,
        provider: "openai",
        include_metadata: false,
        fallback_type: None,
    }
}

fn seen_teams(router: &FakeRouter) -> Vec<Option<String>> {
    router.seen_teams.lock().expect("the fixture lock").clone()
}
