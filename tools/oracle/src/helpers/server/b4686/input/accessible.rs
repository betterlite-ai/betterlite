use betterlite_server::mcp::search_handlers::types::{AccessibleAgent, AccessibleSkill};

use super::{FromFixture, NamedInput};

impl FromFixture<NamedInput> for AccessibleAgent {
    fn from_fixture(input: &NamedInput) -> Self {
        Self {
            agent_id: input.id.clone(),
            agent_name: input.name.clone(),
        }
    }
}

impl FromFixture<NamedInput> for AccessibleSkill {
    fn from_fixture(input: &NamedInput) -> Self {
        Self {
            skill_id: input.id.clone(),
            display_title: Some(input.name.clone()),
            description: input.description.clone(),
        }
    }
}
