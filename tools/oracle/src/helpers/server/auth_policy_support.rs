use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_org_lookup::organization::OrgLookup;
use betterlite_auth::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer};
use betterlite_auth::auth_policy_support::extract::{
    ExtractionError, ToolNameExtractors, ToolRouteCallTypes, extract_request_tool_names,
};
use betterlite_auth::auth_policy_support::fetch::safe_fetch;
use betterlite_auth::auth_policy_support::guardrails::{
    GUARDRAIL_DENIAL, GuardrailCheckError, guardrail_modification_check,
};
use betterlite_auth::auth_policy_support::org::inherit_org_identity;
use betterlite_auth::auth_policy_support::team::{
    TeamFromTokenError, team_obj_from_token, token_can_vouch_for_team,
};
use betterlite_auth::auth_policy_support::tools::{ToolCheckError, check_tools_allowlist};
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::routes::catalog::{CATALOG, RouteCatalog};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_availability::policy::{AllowRequestsOnOutage, PolicyError};
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::{LiteLLMTeamTable, TeamFields};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::failure_hook::lift::call_type_name;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Guardrail {
        body: Value,
        #[serde(default)]
        team: Option<Value>,
    },
    Tools {
        route: String,
        body: Value,
        #[serde(default)]
        token: Option<Value>,
        #[serde(default)]
        team: Option<Value>,
    },
    Extract {
        route: String,
        body: Value,
    },
    TeamFromToken {
        token: Value,
    },
    Vouch {
        token: Value,
        lookup: Lookup,
        #[serde(default)]
        outage: bool,
    },
    OrgInherit {
        token: Value,
        #[serde(default)]
        team: Option<Value>,
        #[serde(default)]
        org: Option<Value>,
        #[serde(default)]
        database: bool,
    },
    SafeFetch {
        failure: Failure,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Lookup {
    TeamNotFound,
    Degraded,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failure {
    Proxy,
    TeamNotFound,
    NoDatabase,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Guardrail { body, team } => guardrail(body, team.as_ref()),
            Input::Tools {
                route,
                body,
                token,
                team,
            } => tools(route, body, token.as_ref(), team.as_ref()),
            Input::Extract { route, body } => extract(route, body),
            Input::TeamFromToken { token } => team_from_token(token),
            Input::Vouch {
                token,
                lookup,
                outage,
            } => vouch(token, *lookup, *outage),
            Input::OrgInherit {
                token,
                team,
                org,
                database,
            } => org_inherit(token, team.as_ref(), org.as_ref(), *database),
            Input::SafeFetch { failure } => fetch_outcome(*failure),
        }
    }
}

fn guardrail(body: &Value, team: Option<&Value>) -> HelperResult {
    let body = body_object(body)?;
    let team = team.map(team_row).transpose()?;
    Ok(match guardrail_modification_check(&body, team.as_ref()) {
        Ok(()) => json!({ "outcome": "allowed" }),
        Err(GuardrailCheckError::Denied) => json!({
            "outcome": "denied",
            "status": 403,
            "detail": { "error": GUARDRAIL_DENIAL },
        }),
        Err(GuardrailCheckError::Metadata(error)) => {
            json!({ "outcome": "error", "message": error.to_string() })
        }
    })
}

fn tools(route: &str, body: &Value, token: Option<&Value>, team: Option<&Value>) -> HelperResult {
    let body = body_object(body)?;
    let token = token.map(token_obj).transpose()?;
    let team = team.map(team_row).transpose()?;
    let extractors = extractors()?;
    let names = match extract_request_tool_names(route, &body, &extractors) {
        Ok(names) => names,
        Err(ExtractionError::NotIterable { .. }) => {
            return Ok(json!({ "outcome": "not_iterable" }));
        }
    };
    Ok(
        match check_tools_allowlist(&body, token.as_ref(), team.as_ref(), route, &extractors) {
            Ok(()) => json!({ "outcome": "allowed", "names": names }),
            Err(ToolCheckError::Denied(exception)) => json!({
                "outcome": "denied",
                "names": names,
                "error_type": exception.error_type,
                "param": exception.param,
                "code": exception.code,
            }),
            Err(ToolCheckError::Extraction(_)) => json!({ "outcome": "not_iterable" }),
        },
    )
}

fn extract(route: &str, body: &Value) -> HelperResult {
    let body = body_object(body)?;
    let extractors = extractors()?;
    Ok(
        match extract_request_tool_names(route, &body, &extractors) {
            Ok(names) => json!({ "outcome": "extracted", "names": names }),
            Err(ExtractionError::NotIterable { .. }) => json!({ "outcome": "not_iterable" }),
        },
    )
}

fn team_from_token(value: &Value) -> HelperResult {
    let token = token_obj(value)?;
    Ok(match team_obj_from_token(&token) {
        Err(TeamFromTokenError::MissingTeamId) => json!({ "error": "missing_team_id" }),
        Err(TeamFromTokenError::NonStringModel { index }) => {
            json!({ "error": "non_string_model", "index": index })
        }
        Ok(cached) => team_view(&cached.team.fields),
    })
}

fn team_view(fields: &TeamFields) -> Value {
    json!({
        "team_id": fields.base.team_id,
        "max_budget": decimal_f64(fields.base.max_budget),
        "soft_budget": decimal_f64(fields.base.soft_budget),
        "model_max_budget": fields.model_max_budget.0,
        "spend": decimal_f64(fields.spend),
        "tpm_limit": fields.base.tpm_limit,
        "rpm_limit": fields.base.rpm_limit,
        "tpd_limit": fields.base.tpd_limit,
        "blocked": fields.base.blocked,
        "models": fields.base.models,
        "metadata": fields.base.metadata,
        "object_permission_id": fields.object_permission_id,
    })
}

fn vouch(value: &Value, lookup: Lookup, outage: bool) -> HelperResult {
    let token = token_obj(value)?;
    let error = match lookup {
        Lookup::TeamNotFound => TeamLookupError::TeamNotFound {
            team_id: "team-x".to_owned(),
        },
        Lookup::Degraded => TeamLookupError::NoDatabase,
    };
    let policy = AllowRequestsOnOutage::Boolean(outage);
    Ok(match token_can_vouch_for_team(&token, &error, &policy) {
        Ok(value) => json!({ "value": value }),
        Err(PolicyError::UnsupportedValue) => json!({ "error": "unsupported_value" }),
    })
}

fn org_inherit(
    value: &Value,
    team: Option<&Value>,
    org: Option<&Value>,
    database: bool,
) -> HelperResult {
    let mut token = token_obj(value)?;
    let team = team.map(team_row).transpose()?;
    let org = org.map(org_row).transpose()?;
    let cache = Arc::new(FakeOrg { org });
    let cache_trait: Arc<dyn OrgCache> = Arc::<FakeOrg>::clone(&cache);
    let store: Option<Arc<dyn OrgStore>> = if database {
        Some(Arc::<FakeOrg>::clone(&cache))
    } else {
        None
    };
    let lookup = OrgLookup {
        store,
        cache: cache_trait,
        in_memory_ttl: 60.0_f64,
    };
    let outage = AllowRequestsOnOutage::Boolean(false);
    runtime()?
        .block_on(inherit_org_identity(
            &mut token,
            team.as_ref(),
            &lookup,
            &outage,
        ))
        .or_value_error()?;
    org_fields(&token)
}

// the token serializes flat, so the identity fields the stage would see come straight off it
fn org_fields(token: &UserAPIKeyAuth) -> HelperResult {
    let serialized = serde_json::to_value(token).or_value_error()?;
    let Value::Object(fields) = serialized else {
        return Err(invalid("the token does not serialize to an object"));
    };
    let mut view = serde_json::Map::new();
    for name in [
        "org_id",
        "organization_alias",
        "organization_max_budget",
        "organization_tpm_limit",
        "organization_rpm_limit",
        "organization_metadata",
    ] {
        view.insert(
            name.to_owned(),
            fields.get(name).cloned().unwrap_or(Value::Null),
        );
    }
    Ok(Value::Object(view))
}

fn fetch_outcome(failure: Failure) -> HelperResult {
    let runtime = runtime()?;
    let re_raised = match failure {
        Failure::Proxy => runtime
            .block_on(safe_fetch("context", async {
                Err::<(), ProxyException>(auth_failure())
            }))
            .is_err(),
        Failure::TeamNotFound => runtime
            .block_on(safe_fetch("context", async {
                Err::<(), TeamLookupError>(TeamLookupError::TeamNotFound {
                    team_id: "team-x".to_owned(),
                })
            }))
            .is_err(),
        Failure::NoDatabase => runtime
            .block_on(safe_fetch("context", async {
                Err::<(), TeamLookupError>(TeamLookupError::NoDatabase)
            }))
            .is_err(),
    };
    Ok(if re_raised {
        json!({ "outcome": "re-raised", "class": failure_name(failure) })
    } else {
        json!({ "outcome": "none" })
    })
}

fn failure_name(failure: Failure) -> &'static str {
    match failure {
        Failure::Proxy => "proxy",
        Failure::TeamNotFound => "team_not_found",
        Failure::NoDatabase => "no_database",
    }
}

fn auth_failure() -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: "denied".to_owned(),
        error_type: "auth_error".to_owned(),
        param: None,
        code: ExceptionCode::Status(403),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()
}

// B207's pinned route catalog answers the same route table LiteLLM's get_call_types_for_route does
#[derive(Debug)]
struct CatalogRoutes {
    catalog: RouteCatalog,
}

impl ToolRouteCallTypes for CatalogRoutes {
    fn call_types_for_route(&self, route: &str) -> Vec<String> {
        let Ok(Some(types)) = self.catalog.call_types_for(route) else {
            return Vec::new();
        };
        types
            .iter()
            .filter_map(|call_type| call_type_name(*call_type))
            .collect()
    }
}

fn extractors() -> Result<ToolNameExtractors, Box<HelperError>> {
    let catalog = RouteCatalog::parse(CATALOG).or_value_error()?;
    Ok(ToolNameExtractors::builtin(Arc::new(CatalogRoutes {
        catalog,
    })))
}

fn body_object(value: &Value) -> Result<JsonObject, Box<HelperError>> {
    serde_json::from_value(value.clone()).or_value_error()
}

fn team_row(value: &Value) -> Result<LiteLLMTeamTable, Box<HelperError>> {
    serde_json::from_value(value.clone()).or_value_error()
}

fn org_row(value: &Value) -> Result<OrganizationTable, Box<HelperError>> {
    serde_json::from_value(value.clone()).or_value_error()
}

fn token_obj(value: &Value) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let JsonData::Object(fields) = serde_json::from_value(value.clone()).or_value_error()? else {
        return Err(invalid("the token must be an object"));
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn decimal_f64(value: Option<rust_decimal::Decimal>) -> Option<f64> {
    value.and_then(|decimal| decimal.to_f64())
}

// the org comes from the cache, so the repository answers nothing and is only there to mark the database present
struct FakeOrg {
    org: Option<OrganizationTable>,
}

#[async_trait]
impl OrgCache for FakeOrg {
    async fn read_org(
        &self,
        _key: &str,
        _local_only: bool,
    ) -> Result<Option<OrganizationTable>, AuthCacheError> {
        Ok(self.org.clone())
    }

    async fn write_org(
        &self,
        _key: &str,
        _org: &OrganizationTable,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    async fn read_project(&self, _key: &str) -> Result<Option<CachedProject>, AuthCacheError> {
        Ok(None)
    }

    async fn write_project(
        &self,
        _key: &str,
        _project: &CachedProject,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

#[async_trait]
impl OrgStore for FakeOrg {
    async fn find_org(&self, _org_id: &str, _include_budget: bool) -> StoreAnswer<Option<String>> {
        Ok(None)
    }

    async fn find_by_alias(&self, _org_alias: &str) -> StoreAnswer<Vec<String>> {
        Ok(Vec::new())
    }

    async fn find_project(&self, _project_id: &str) -> StoreAnswer<Option<String>> {
        Ok(None)
    }
}
