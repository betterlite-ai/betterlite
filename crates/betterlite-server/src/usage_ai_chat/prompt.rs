use chrono::NaiveDate;

pub const SYSTEM_PROMPT_BASE: &str = concat!(
    "You are an AI assistant embedded in the LiteLLM Usage dashboard. ",
    "You help users understand their LLM API spend and usage data.\n\n",
    "ALWAYS call the appropriate tool(s) first to fetch data before answering. ",
    "You may call multiple tools if the question spans different dimensions.\n\n",
    "Guidelines:\n",
    "- Be concise and specific. Use exact numbers from the data.\n",
    "- Format costs as dollar amounts (e.g. $12.34).\n",
    "- When comparing entities, show a ranked list.\n",
    "- If data is empty or no results found, say so clearly.\n",
    "- Do not hallucinate data — only use what the tools return.\n",
    "- Today's date will be provided below. Use it to interpret relative dates ",
    "like 'this week', 'this month', 'last 7 days', etc."
);

const TOOL_DESCRIPTIONS_ADMIN: &str = concat!(
    "You have access to these tools:\n",
    "- `get_usage_data`: Global/user-level usage (spend, models, providers, API keys)\n",
    "- `get_team_usage_data`: Team-level usage breakdown\n",
    "- `get_tag_usage_data`: Tag-level usage breakdown\n\n"
);

const TOOL_DESCRIPTIONS_BASE: &str = concat!(
    "You have access to this tool:\n",
    "- `get_usage_data`: Your usage data (spend, models, providers, API keys)\n\n"
);

#[must_use]
pub fn build_system_prompt(is_admin: bool, today: NaiveDate) -> String {
    let tool_descriptions = if is_admin {
        TOOL_DESCRIPTIONS_ADMIN
    } else {
        TOOL_DESCRIPTIONS_BASE
    };
    format!(
        "{SYSTEM_PROMPT_BASE}\n\n{tool_descriptions}Today's date: {}",
        today.format("%Y-%m-%d")
    )
}
