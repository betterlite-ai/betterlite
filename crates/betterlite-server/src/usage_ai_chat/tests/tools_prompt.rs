#[test]
fn tools_admin_gets_the_three_usage_tools() {
    let tools = tools_for_role(true);
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool.function.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "get_usage_data",
            "get_team_usage_data",
            "get_tag_usage_data"
        ]
    );
    assert!(tools.iter().all(|tool| tool.kind == ToolKind::Function));
    assert!(
        tools
            .iter()
            .all(|tool| tool.function.parameters.kind == SchemaKind::Object)
    );
}

#[test]
fn tools_user_gets_only_the_global_tool() {
    let tools = tools_for_role(false);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].function.name, "get_usage_data");
}

#[test]
fn tools_descriptions_match_litellm() {
    let tools = tools_for_role(true);
    assert_eq!(
        tools[0].function.description,
        concat!(
            "Fetch aggregated global usage/spend data. Returns daily spend, ",
            "token counts, request counts, and breakdowns by model, provider, ",
            "and API key. Use for overall spend, top models, top providers."
        )
    );
    assert_eq!(
        tools[1].function.description,
        concat!(
            "Fetch usage/spend data broken down by team. Use for questions ",
            "like 'which team spends the most' or 'show me team X usage'."
        )
    );
    assert_eq!(
        tools[2].function.description,
        concat!(
            "Fetch usage/spend data broken down by tag. Tags are labels ",
            "attached to requests (features, environments, credentials)."
        )
    );
}

#[test]
fn tools_schemas_match_litellm() {
    let tools = tools_for_role(true);
    let global = &tools[0].function.parameters;
    assert_eq!(
        global
            .properties
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["start_date", "end_date", "user_id"]
    );
    assert_eq!(global.required, ["start_date", "end_date"]);
    assert_eq!(
        global.properties["start_date"].description,
        "Start date in YYYY-MM-DD format"
    );
    assert_eq!(
        global.properties["end_date"].description,
        "End date in YYYY-MM-DD format"
    );
    assert_eq!(
        global.properties["user_id"].description,
        "Optional user ID filter. Omit for global view."
    );
}

#[test]
fn tools_team_and_tag_schemas_match_litellm() {
    let tools = tools_for_role(true);
    let team = &tools[1].function.parameters;
    assert_eq!(
        team.properties
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["start_date", "end_date", "team_ids"]
    );
    assert_eq!(
        team.properties["team_ids"].description,
        "Optional comma-separated team IDs. Omit for all teams."
    );
    let tag = &tools[2].function.parameters;
    assert_eq!(
        tag.properties
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["start_date", "end_date", "tags"]
    );
    assert_eq!(
        tag.properties["tags"].description,
        "Optional comma-separated tag names. Omit for all tags."
    );
    assert!(tools.iter().all(|tool| {
        tool.function.parameters.required == ["start_date", "end_date"]
            && tool
                .function
                .parameters
                .properties
                .values()
                .all(|property| property.kind == SchemaKind::String)
    }));
}

#[test]
fn prompt_admin_names_every_tool_and_the_date() {
    assert_eq!(
        build_system_prompt(true, on(2026, 11, 16)),
        format!(
            "{SYSTEM_PROMPT_BASE}\n\nYou have access to these tools:\n- `get_usage_data`: Global/user-level usage (spend, models, providers, API keys)\n- `get_team_usage_data`: Team-level usage breakdown\n- `get_tag_usage_data`: Tag-level usage breakdown\n\nToday's date: 2026-11-16"
        )
    );
}

#[test]
fn prompt_user_names_only_the_global_tool() {
    assert_eq!(
        build_system_prompt(false, on(2026, 1, 2)),
        format!(
            "{SYSTEM_PROMPT_BASE}\n\nYou have access to this tool:\n- `get_usage_data`: Your usage data (spend, models, providers, API keys)\n\nToday's date: 2026-01-02"
        )
    );
}

#[test]
fn prompt_base_text_matches_litellm() {
    assert_eq!(
        SYSTEM_PROMPT_BASE,
        concat!(
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
        )
    );
}
