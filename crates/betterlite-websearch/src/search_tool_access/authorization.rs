use std::error::Error;
use std::future::Future;

use super::hook_kwargs::{ParentOtelSpan, SearchCaller, SearchHookKwargs, caller_from_kwargs};
use super::tool_selection::{SearchToolConfig, selected_search_tool_name};

pub trait SearchToolGrants: Send + Sync {
    type Team;
    type Error: Error + Send + Sync + 'static;

    fn check_key(
        &self,
        name: &str,
        caller: &SearchCaller,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn load_team(
        &self,
        team_id: &str,
        parent_otel_span: Option<&ParentOtelSpan>,
    ) -> impl Future<Output = Result<Self::Team, Self::Error>> + Send;

    fn check_team(
        &self,
        name: &str,
        team: &Self::Team,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum SearchToolAccessError<GrantError>
where
    GrantError: Error + 'static,
{
    #[error("the search tool grants check failed")]
    Grants(#[source] GrantError),
}

pub struct SearchToolAuthorizer<Grants> {
    pub grants_port: Grants,
}

impl<Grants: SearchToolGrants> SearchToolAuthorizer<Grants> {
    pub async fn authorize(
        &self,
        search_tool: &SearchToolConfig,
        kwargs: Option<&SearchHookKwargs>,
    ) -> Result<(), SearchToolAccessError<Grants::Error>> {
        let Some(name) = selected_search_tool_name(Some(search_tool)) else {
            return Ok(());
        };
        let Some(caller) = caller_from_kwargs(kwargs) else {
            return Ok(());
        };
        self.grants_port
            .check_key(name, caller)
            .await
            .map_err(SearchToolAccessError::Grants)?;
        let Some(team_id) = caller
            .team_id
            .as_deref()
            .filter(|team_id| !team_id.is_empty())
        else {
            return Ok(());
        };
        let team = self
            .grants_port
            .load_team(team_id, caller.parent_otel_span.as_ref())
            .await
            .map_err(SearchToolAccessError::Grants)?;
        self.grants_port
            .check_team(name, &team)
            .await
            .map_err(SearchToolAccessError::Grants)?;
        Ok(())
    }
}
