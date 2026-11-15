use crate::bedrock_messages_beta::tests::support::{Call, Fixture, Session, headers, list};

const SEARCH: &str = r#"[{"type": "tool_search_tool_regex_20251119", "name": "tool_search"}]"#;
const WITH_EXAMPLES: &str = r#"[{"type": "tool_search_tool_regex_20251119", "name": "tool_search", "input_examples": [{"query": "x"}]}]"#;
const PROGRAMMATIC: &str = r#"[{"type": "tool_search_tool_bm25_20251119", "name": "tool_search", "allowed_callers": ["code_execution_20250825"]}]"#;
const ADVANCED: &str = r#"{"anthropic-beta": "advanced-tool-use-2025-11-20"}"#;
const SEARCH_BETA: &str = r#"{"anthropic-beta": "tool-search-tool-2025-10-19"}"#;

const TOOL_SEARCH_BETA: &str = "tool-search-tool-2025-10-19";
const TOOL_EXAMPLES_BETA: &str = "tool-examples-2025-10-29";

fn betas(tools: &str, header: &str, model: &str, provider: Option<&str>) -> Vec<String> {
    let fixture = Fixture::new();
    let mut session = Session::new(&fixture);
    let mut call = Call::default();
    call.model = model.to_owned();
    call.tools = list(tools);
    call.header_values = headers(header);
    call.subclass_provider = provider.map(str::to_owned);
    session
        .betas(call.request())
        .expect("the beta headers resolve")
}

pub(crate) fn run() {
    // supported: the advanced beta is discarded and the Bedrock tool-search beta replaces it
    assert_eq!(
        betas(SEARCH, ADVANCED, "fixture-tool-search", None),
        [TOOL_SEARCH_BETA]
    );
    // unsupported: the discard still happens, and no tool-search beta is added back
    assert!(betas(SEARCH, ADVANCED, "claude-sonnet-4-5", None).is_empty());
    // tool examples ride along when the provider keeps them
    assert_eq!(
        betas(
            SEARCH,
            ADVANCED,
            "fixture-tool-search",
            Some("bedrock_mantle")
        ),
        [TOOL_EXAMPLES_BETA, TOOL_SEARCH_BETA]
    );
    // input examples or programmatic calls leave the set untouched
    assert_eq!(
        betas(
            WITH_EXAMPLES,
            ADVANCED,
            "fixture-tool-search",
            Some("bedrock_mantle")
        ),
        [TOOL_SEARCH_BETA]
    );
    assert_eq!(
        betas(
            PROGRAMMATIC,
            ADVANCED,
            "fixture-tool-search",
            Some("bedrock_mantle")
        ),
        [TOOL_SEARCH_BETA]
    );
    // a user tool-search beta the adjust never added still pulls in the examples beta
    assert_eq!(
        betas(
            "[]",
            SEARCH_BETA,
            "fixture-tool-search",
            Some("bedrock_mantle")
        ),
        [TOOL_EXAMPLES_BETA, TOOL_SEARCH_BETA]
    );
    // without a tool-search tool the set stays untouched
    assert_eq!(
        betas("[]", ADVANCED, "fixture-tool-search", Some("anthropic")),
        ["advanced-tool-use-2025-11-20"]
    );
    // the adjust discards the user's advanced beta before the provider filter sees it
    assert!(betas(SEARCH, ADVANCED, "fixture-tool-search", Some("anthropic")).is_empty());
}
