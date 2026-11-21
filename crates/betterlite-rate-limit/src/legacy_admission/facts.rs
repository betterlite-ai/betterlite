use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::budget_primitives::throttle::throttled_limit;
use betterlite_auth::request_quota_view::RequestQuotaViewExt as _;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::session_token_codec::record::SessionRecord;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_principal_metadata::principal::logged_api_key;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::rate_limiters::legacy_rate_limit_counters::objects::CacheObjectKeys;

// Python's sys.maxsize: every limit LiteLLM leaves absent is this value
pub(super) const UNLIMITED: i64 = i64::MAX;

// what an f-string writes for a missing api_key or model, and so the text the model counter id carries
pub(super) const NONE_TEXT: &str = "None";

// the one cache key in the global cap's step, shared by its read, its write and the six-key read
pub(super) const GLOBAL_KEY: &str = "global_max_parallel_requests";

// the limits and ids the hook's later steps key their counters by
pub(super) struct AdmissionFacts {
    pub(super) api_key: Option<String>,
    pub(super) max_parallel_requests: i64,
    pub(super) tpm_limit: i64,
    pub(super) rpm_limit: i64,
    pub(super) user_id: Option<String>,
    pub(super) user_tpm_limit: i64,
    pub(super) user_rpm_limit: i64,
    pub(super) team_id: Option<String>,
    pub(super) team_tpm_limit: i64,
    pub(super) team_rpm_limit: i64,
    pub(super) end_user_id: Option<String>,
    pub(super) end_user_tpm_limit: i64,
    pub(super) end_user_rpm_limit: i64,
}

impl AdmissionFacts {
    #[must_use]
    pub(super) fn read(principal: &UserAPIKeyAuth) -> Self {
        let view = principal.request_quota_view();
        let account = account_limits(principal);
        // B1106's logged_api_key reads these three facts; the auth record keeps them private
        let record = SessionRecord::from_auth(principal);
        let facts = principal_facts(account.hashed_api_key, record);
        Self {
            api_key: logged_api_key(&facts).map(str::to_owned),
            max_parallel_requests: view.max_parallel_requests.unwrap_or(UNLIMITED),
            tpm_limit: throttled_limit(view.tpm_limit, view.throttle_pct).unwrap_or(UNLIMITED),
            rpm_limit: throttled_limit(view.rpm_limit, view.throttle_pct).unwrap_or(UNLIMITED),
            user_id: view.user_id.map(str::to_owned),
            user_tpm_limit: view.user_tpm_limit.unwrap_or(UNLIMITED),
            user_rpm_limit: view.user_rpm_limit.unwrap_or(UNLIMITED),
            team_id: account.team_id.map(str::to_owned),
            team_tpm_limit: view.team_tpm_limit.unwrap_or(UNLIMITED),
            team_rpm_limit: view.team_rpm_limit.unwrap_or(UNLIMITED),
            end_user_id: view.end_user_id.map(str::to_owned),
            end_user_tpm_limit: view.end_user_tpm_limit.unwrap_or(UNLIMITED),
            end_user_rpm_limit: view.end_user_rpm_limit.unwrap_or(UNLIMITED),
        }
    }
}

// only the three facts logged_api_key reads are filled; every other budget field is irrelevant to the hook
#[must_use]
fn principal_facts(api_key_hash: Option<&str>, record: SessionRecord) -> PrincipalFacts {
    PrincipalFacts {
        api_key_hash: api_key_hash.map(str::to_owned),
        is_session_token: record.is_session_token,
        key_alias: record.key_alias,
        spend: Decimal::ZERO,
        max_budget: None,
        user_spend: None,
        user_max_budget: None,
        team_spend: None,
        team_max_budget: None,
        end_user_max_budget: None,
        budget_reset_at: None,
        team_id: None,
        team_alias: None,
        project_id: None,
        project_alias: None,
        org_id: None,
        organization_alias: None,
        user_id: None,
        user_email: None,
        end_user_id: None,
        request_route: None,
        agent_id: None,
        invoked_agent_id: None,
        managed_agent_context: None,
        budget_reservation: None,
        matched_model_access_groups: Vec::new(),
        metadata: IndexMap::new(),
        team_metadata: None,
        project_metadata: None,
        organization_metadata: None,
    }
}

// the counter ids the hook builds: `{id}::{minute}::request_count`, with the model id keeping f-string None text
pub(super) struct CounterIds {
    pub(super) key: Option<String>,
    pub(super) key_model: Option<String>,
    pub(super) key_model_text: String,
    pub(super) user: Option<String>,
    pub(super) team: Option<String>,
    pub(super) end_user: Option<String>,
}

impl CounterIds {
    // the six-key batch read: each id is None exactly when an f-string condition in the hook is False
    #[must_use]
    pub(super) fn read_keys(&self, global_present: bool) -> CacheObjectKeys {
        CacheObjectKeys {
            current_global_requests: global_present.then(|| String::from(GLOBAL_KEY)),
            request_count_api_key: self.key.clone(),
            request_count_api_key_model: self.key_model.clone(),
            request_count_user_id: self.user.clone(),
            request_count_team_id: self.team.clone(),
            request_count_end_user_id: self.end_user.clone(),
        }
    }
}

#[must_use]
pub(super) fn counter_ids(facts: &AdmissionFacts, model: Option<&str>, minute: &str) -> CounterIds {
    let api_key_text = facts.api_key.as_deref().unwrap_or(NONE_TEXT);
    let model_text = model.unwrap_or(NONE_TEXT);
    CounterIds {
        key: facts
            .api_key
            .as_ref()
            .map(|api_key| format!("{api_key}::{minute}::request_count")),
        key_model: facts
            .api_key
            .as_ref()
            .zip(model)
            .map(|(api_key, model_name)| {
                format!("{api_key}::{model_name}::{minute}::request_count")
            }),
        key_model_text: format!("{api_key_text}::{model_text}::{minute}::request_count"),
        user: facts
            .user_id
            .as_ref()
            .map(|user_id| format!("{user_id}::{minute}::request_count")),
        team: facts
            .team_id
            .as_ref()
            .map(|team_id| format!("{team_id}::{minute}::request_count")),
        end_user: facts
            .end_user_id
            .as_ref()
            .map(|end_user_id| format!("{end_user_id}::{minute}::request_count")),
    }
}
