mod terminal;

use crate::loader::EnvironmentLookup;
use crate::sections::chatgpt_headers::Settings;

pub(crate) fn resolve(lookup: &dyn EnvironmentLookup) -> Settings {
    Settings {
        originator_override: lookup.lookup("CHATGPT_ORIGINATOR"),
        user_agent_override: lookup.lookup("CHATGPT_USER_AGENT"),
        user_agent_suffix: lookup.lookup("CHATGPT_USER_AGENT_SUFFIX"),
        default_instructions_override: lookup.lookup("CHATGPT_DEFAULT_INSTRUCTIONS"),
        terminal: terminal::resolve(lookup),
    }
}
