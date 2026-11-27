use betterlite_server::mcp::search_handlers::types::{AccessibleAgent, AccessibleSkill};

use super::{FromFixture, NamedInput};

impl FromFixture<NamedInput> for AccessibleAgent {
    fn from_fixture(agent: &NamedInput) -> Self {
        Self {
            agent_id: agent.id.clone(),
            agent_name: agent.name.clone(),
        }
    }
}

impl FromFixture<NamedInput> for AccessibleSkill {
    fn from_fixture(skill: &NamedInput) -> Self {
        Self {
            skill_id: skill.id.clone(),
            display_title: Some(skill.name.clone()),
            description: skill.description.clone(),
        }
    }
}
