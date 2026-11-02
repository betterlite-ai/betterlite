mod terminal;

use crate::loader::EnvironmentLookup;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatGptHeaderEnvironment {
    pub(crate) originator_override: Option<String>,
    pub(crate) user_agent_override: Option<String>,
    pub(crate) user_agent_suffix: Option<String>,
    pub(crate) default_instructions_override: Option<String>,
    pub(crate) terminal: String,
}

pub(crate) fn resolve(lookup: &dyn EnvironmentLookup) -> ChatGptHeaderEnvironment {
    ChatGptHeaderEnvironment {
        originator_override: lookup.lookup("CHATGPT_ORIGINATOR"),
        user_agent_override: lookup.lookup("CHATGPT_USER_AGENT"),
        user_agent_suffix: lookup.lookup("CHATGPT_USER_AGENT_SUFFIX"),
        default_instructions_override: lookup.lookup("CHATGPT_DEFAULT_INSTRUCTIONS"),
        terminal: terminal::resolve(lookup),
    }
}

mod tests;
