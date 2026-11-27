use betterlite_server::agent_endpoints::search::{AgentSearchResult, SearchableSkill};

use super::{AGENT_SCORE, FromFixture, NamedInput};

impl FromFixture<NamedInput> for AgentSearchResult {
    fn from_fixture(agent: &NamedInput) -> Self {
        let id = &agent.id;
        Self {
            agent_id: id.clone(),
            agent_name: agent.name.clone(),
            description: format!("agent description of {id}"),
            skills: vec![SearchableSkill {
                name: format!("skill-{id}"),
                description: format!("skill description of {id}"),
                tags: vec!["seeded".to_owned()],
            }],
            score: AGENT_SCORE,
        }
    }
}
