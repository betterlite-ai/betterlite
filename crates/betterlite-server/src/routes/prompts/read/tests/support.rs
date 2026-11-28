use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::prompt_spec::{PromptInfo, PromptLiteLLMParams, PromptSpec};
use betterlite_prompts::registry::prompt_endpoint_contracts::rows::{
    PromptRow, PromptRowData, PromptRowJson,
};
use betterlite_prompts::registry::prompt_endpoint_contracts::table::{
    PromptColumn, PromptRowFilter, PromptRowQuery, PromptRowUpdate, PromptTableActions,
};
use betterlite_prompts::registry::prompt_registry::InMemoryPromptRegistry;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;
use thiserror::Error;
use tokio::sync::RwLock;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::routes::prompts::read::{PromptReadState, routes};

#[derive(Debug, Error)]
#[error("the fake prompt table reports a failure")]
pub(super) struct FakeTableFailure;

pub(super) struct FakePromptTable {
    rows: Vec<PromptRow>,
    failing: bool,
    queries: Mutex<Vec<PromptRowQuery>>,
}

impl FakePromptTable {
    pub(super) fn answering(rows: Vec<PromptRow>) -> Arc<Self> {
        Arc::new(Self {
            rows,
            failing: false,
            queries: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn failing() -> Arc<Self> {
        Arc::new(Self {
            rows: Vec::new(),
            failing: true,
            queries: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn queries(&self) -> Vec<PromptRowQuery> {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PromptTableActions for FakePromptTable {
    type Failure = FakeTableFailure;

    fn find_many(
        &self,
        query: PromptRowQuery,
    ) -> impl Future<Output = Result<Vec<PromptRow>, Self::Failure>> + Send {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(query.clone());
        if self.failing {
            return ready(Err(FakeTableFailure));
        }
        let mut rows: Vec<PromptRow> = self
            .rows
            .iter()
            .filter(|row| matches_filter(row, &query.filter))
            .cloned()
            .collect();
        if query
            .order
            .iter()
            .any(|order| order.column == PromptColumn::Version && order.descending)
        {
            rows.sort_by_key(|row| Reverse(row.version));
        }
        if let Some(take) = query.take {
            rows.truncate(usize::try_from(take).unwrap_or_default());
        }
        ready(Ok(rows))
    }

    fn create(
        &self,
        _data: PromptRowData,
    ) -> impl Future<Output = Result<PromptRow, Self::Failure>> + Send {
        ready(Err(FakeTableFailure))
    }

    fn update(
        &self,
        _filter: PromptRowFilter,
        _data: PromptRowUpdate,
    ) -> impl Future<Output = Result<Option<PromptRow>, Self::Failure>> + Send {
        ready(Err(FakeTableFailure))
    }

    fn delete_many(
        &self,
        _filter: PromptRowFilter,
    ) -> impl Future<Output = Result<u64, Self::Failure>> + Send {
        ready(Err(FakeTableFailure))
    }
}

fn matches_filter(row: &PromptRow, filter: &PromptRowFilter) -> bool {
    filter.id.as_ref().is_none_or(|wanted| &row.id == wanted)
        && filter
            .prompt_id
            .as_ref()
            .is_none_or(|wanted| &row.prompt_id == wanted)
        && filter.version.is_none_or(|wanted| row.version == wanted)
        && filter
            .environment
            .as_ref()
            .is_none_or(|wanted| &row.environment == wanted)
}

fn context(subject: &str, roles: &[Role], source_key: SourceKey) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: roles.to_vec(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key,
        },
    }
}

pub(super) fn role_context(roles: &[Role]) -> AuthContext {
    context("role-user", roles, SourceKey::default())
}

pub(super) fn master_context() -> AuthContext {
    master_caller()
}

pub(super) fn ordinary_context() -> AuthContext {
    context("ordinary-user", &[], SourceKey::default())
}

pub(super) fn key_context(key_fields: &str) -> AuthContext {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(key_fields).expect("the key fixture parses");
    let key =
        UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key fixture constructs");
    context("key-owner", &[], SourceKey(Some(key)))
}

pub(super) fn state(
    prompts: Arc<RwLock<InMemoryPromptRegistry>>,
    table: Option<Arc<FakePromptTable>>,
) -> PromptReadState<FakePromptTable> {
    PromptReadState { prompts, table }
}

pub(super) fn registry(specs: Vec<PromptSpec>) -> Arc<RwLock<InMemoryPromptRegistry>> {
    let in_memory_prompts = specs
        .into_iter()
        .map(|spec| (spec.prompt_id.clone(), spec))
        .collect();
    Arc::new(RwLock::new(InMemoryPromptRegistry {
        in_memory_prompts,
        prompt_id_to_custom_prompt: IndexMap::new(),
    }))
}

pub(super) fn config_prompt(prompt_id: &str, environment: &str) -> PromptSpec {
    PromptSpec {
        prompt_id: prompt_id.to_owned(),
        litellm_params: PromptLiteLLMParams {
            prompt_id: Some(prompt_id.to_owned()),
            prompt_integration: "dotprompt".to_owned(),
            api_base: None,
            api_key: None,
            provider_specific_query_params: None,
            ignore_model: false,
            ignore_optional_params: false,
            dotprompt_content: None,
            extra: IndexMap::new(),
        },
        prompt_info: PromptInfo::default(),
        created_at: None,
        updated_at: None,
        version: None,
        environment: environment.to_owned(),
        created_by: None,
    }
}

pub(super) fn dotprompt_prompt(
    prompt_id: &str,
    environment: &str,
    dotprompt_content: &str,
) -> PromptSpec {
    let mut spec = config_prompt(prompt_id, environment);
    spec.litellm_params.dotprompt_content = Some(dotprompt_content.to_owned());
    spec
}

pub(super) fn keyed_prompt(prompt_id: &str, api_key: &str) -> PromptSpec {
    let mut spec = config_prompt(prompt_id, "development");
    spec.litellm_params.api_key = Some(SecretString::from(api_key));
    spec
}

pub(super) fn stored_prompt(prompt_id: &str, version: i64, environment: &str) -> PromptRow {
    stored_prompt_with_content(prompt_id, version, environment, "")
}

pub(super) fn stored_prompt_with_content(
    prompt_id: &str,
    version: i64,
    environment: &str,
    dotprompt_content: &str,
) -> PromptRow {
    let params = serde_json::to_string(&StoredParams {
        prompt_id,
        prompt_integration: "dotprompt",
        dotprompt_content,
    })
    .expect("the stored params serialize");
    PromptRow {
        id: format!("{prompt_id}-{environment}-{version}"),
        prompt_id: prompt_id.to_owned(),
        version,
        environment: environment.to_owned(),
        created_by: None,
        created_at: timestamp(),
        updated_at: timestamp(),
        litellm_params: PromptRowJson::Text(params),
        prompt_info: None,
    }
}

#[derive(Serialize)]
struct StoredParams<'text> {
    prompt_id: &'text str,
    prompt_integration: &'text str,
    dotprompt_content: &'text str,
}

fn timestamp() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(0, 0).expect("the fixed timestamp is valid")
}

pub(super) async fn call(
    state: PromptReadState<FakePromptTable>,
    uri: &str,
    context: AuthContext,
) -> (StatusCode, JsonValue) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .extension(context)
        .body(Body::empty())
        .expect("the request builds");
    let router: Router = routes(state);
    let response = router.oneshot(request).await.expect("the router answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body reads");
    let body = serde_json::from_slice(&bytes).expect("the body is JSON");
    (status, body)
}
