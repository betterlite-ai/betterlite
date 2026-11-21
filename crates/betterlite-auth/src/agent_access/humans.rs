use std::collections::BTreeSet;

use super::error::AgentAccessError;
use super::listing::granted_ids;
use super::reads::{GrantSources, HumanSources, TargetAgents};
use super::resolver::{AgentAccessResolver, resolve_agent_access};

mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HumanGrant {
    // None is the user's own grants, Some is a team the user is a member of
    pub team_id: Option<String>,
    pub agent_ids: BTreeSet<String>,
}

pub struct DelegatedTeamRequest<'request> {
    pub user_id: Option<&'request str>,
    pub agent_id: &'request str,
    pub team_id: Option<&'request str>,
    pub explicit_team: bool,
    // filters only the granting teams; the sources themselves are always loaded unfiltered
    pub allowed_team_ids: Option<&'request BTreeSet<String>>,
}

pub async fn verified_human_sources<Grants, Targets, Humans>(
    resolver: &AgentAccessResolver<Grants, Targets, Humans>,
    user_id: Option<&str>,
    allowed_team_ids: Option<&BTreeSet<String>>,
) -> Result<Vec<HumanGrant>, AgentAccessError>
where
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
{
    let Some(subject) = user_id else {
        return Ok(Vec::new());
    };
    let human = resolver.humans.reload_admitted_user(subject, true).await?;
    let sources = resolver
        .humans
        .admitted_subject_sources(&human, allowed_team_ids)
        .await?;
    let mut grants = Vec::new();
    for source in sources {
        // a source resolves through the same path that may reach a managed key again, so the call is boxed
        let access = Box::pin(resolve_agent_access(resolver, Some(&source), true)).await?;
        grants.push(HumanGrant {
            team_id: source.view.token.team_id,
            agent_ids: granted_ids(access),
        });
    }
    Ok(grants)
}

pub async fn verified_human_agent_grants<
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
>(
    resolver: &AgentAccessResolver<Grants, Targets, Humans>,
    user_id: Option<&str>,
    team_id: Option<&str>,
) -> Result<BTreeSet<String>, AgentAccessError> {
    // Some of an empty set admits no team source at all, unlike None
    let allowed_team_ids: BTreeSet<String> = team_id.map(str::to_owned).into_iter().collect();
    let sources = verified_human_sources(resolver, user_id, Some(&allowed_team_ids)).await?;
    Ok(sources
        .into_iter()
        .filter(|source| source.team_id.is_none() || source.team_id.as_deref() == team_id)
        .flat_map(|source| source.agent_ids)
        .collect())
}

pub async fn resolve_delegated_agent_team<
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
>(
    resolver: &AgentAccessResolver<Grants, Targets, Humans>,
    request: &DelegatedTeamRequest<'_>,
) -> Result<Option<String>, AgentAccessError> {
    let sources = verified_human_sources(resolver, request.user_id, None).await?;
    let user_grant = sources
        .iter()
        .any(|source| source.team_id.is_none() && source.agent_ids.contains(request.agent_id));
    if user_grant {
        return Ok(request.team_id.map(str::to_owned));
    }
    let granting_teams: BTreeSet<&str> = sources
        .iter()
        .filter(|source| source.agent_ids.contains(request.agent_id))
        .filter_map(|source| source.team_id.as_deref())
        .filter(|team| {
            request
                .allowed_team_ids
                .is_none_or(|allowed| allowed.contains(*team))
        })
        .collect();
    if let Some(team) = request.team_id.filter(|team| granting_teams.contains(team)) {
        return Ok(Some(String::from(team)));
    }
    if !request.explicit_team
        && let Some(smallest) = granting_teams.first()
    {
        return Ok(Some(String::from(*smallest)));
    }
    Err(AgentAccessError::TeamNotGranted)
}
