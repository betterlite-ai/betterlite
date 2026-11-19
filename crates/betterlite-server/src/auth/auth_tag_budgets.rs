use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_tag_budgets::TagBudgetError;
use betterlite_auth::auth_tag_budgets::groups::ModelAccessGroupBudget;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use crate::auth::auth_identity::AuthContext;
use crate::state::ServerState;

mod tests;

// The batch reads both budget checks share, so the stage resolves from whatever cache and rows the proxy wired.
#[async_trait]
pub trait TagBudgetLookup: Send + Sync {
    async fn tag_objects(
        &self,
        names: &[String],
        database: bool,
    ) -> Result<BTreeMap<String, TagTable>, TagBudgetError>;

    async fn group_budgets(
        &self,
        names: &[String],
        database: bool,
    ) -> Result<BTreeMap<String, ModelAccessGroupBudget>, TagBudgetError>;
}

// What the request asks to price: the tags it carries and the model access groups that matched it.
#[derive(Default)]
pub struct TagBudgetRule {
    pub tag_names: Vec<String>,
    pub model_access_group_names: Vec<String>,
    pub lookup: Option<Arc<dyn TagBudgetLookup>>,
}

impl fmt::Debug for TagBudgetRule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TagBudgetRule")
            .field("tag_names", &self.tag_names)
            .field("model_access_group_names", &self.model_access_group_names)
            .field("lookup", &self.lookup.is_some())
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct TagBudgetStageInput {
    pub context: AuthContext,
    pub rule: Arc<TagBudgetRule>,
    pub database: bool,
}

pub(crate) fn rule_for_request(request: &Request<Body>) -> Arc<TagBudgetRule> {
    request
        .extensions()
        .get::<Arc<TagBudgetRule>>()
        .map_or_else(|| Arc::new(TagBudgetRule::default()), Arc::clone)
}

pub(crate) fn database_for_request(request: &Request<Body>) -> bool {
    request
        .extensions()
        .get::<Arc<ServerState>>()
        .is_some_and(|state| state.auth_tag_budgets.database)
}

// LiteLLM's common_checks: both batch reads fail open on a fetch error, so only a cache failure reaches here.
pub(crate) async fn prefetch(stage: &TagBudgetStageInput) -> Result<(), Box<ProxyException>> {
    let rule = &stage.rule;
    let Some(lookup) = rule.lookup.as_deref() else {
        return Ok(());
    };
    if !stage.database || (rule.tag_names.is_empty() && rule.model_access_group_names.is_empty()) {
        return Ok(());
    }
    resolve(lookup, rule, stage.database)
        .await
        .map_err(|error| Box::new(auth_error(&error)))
}

// The tag read runs first: LiteLLM gathers both checks and raises the tag failure before the group one.
async fn resolve(
    lookup: &dyn TagBudgetLookup,
    rule: &TagBudgetRule,
    database: bool,
) -> Result<(), TagBudgetError> {
    if !rule.tag_names.is_empty() {
        lookup.tag_objects(&rule.tag_names, database).await?;
    }
    if !rule.model_access_group_names.is_empty() {
        lookup
            .group_budgets(&rule.model_access_group_names, database)
            .await?;
    }
    Ok(())
}

// A bare exception during auth becomes auth_error with param "None" and status 401.
fn auth_error(error: &TagBudgetError) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: format!("Authentication Error, {error}"),
        error_type: "auth_error".to_owned(),
        param: Some("None".to_owned()),
        code: ExceptionCode::Status(401),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}
