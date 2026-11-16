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
    let mut call = Call::new(model);
    call.tools = list(tools);
    call.header_values = headers(header);
    call.subclass_provider = provider.map(str::to_owned);
    session
        .betas(call.request())
        .expect("the beta headers resolve")
}

// the advanced beta is discarded, and the Bedrock tool-search beta replaces it only where supported
fn replaces_advanced_beta() {
    assert_eq!(
        betas(SEARCH, ADVANCED, "fixture-tool-search", None),
        [TOOL_SEARCH_BETA]
    );
    assert!(betas(SEARCH, ADVANCED, "claude-sonnet-4-5", None).is_empty());
}

fn mantle(tools: &str, header: &str) -> Vec<String> {
    betas(tools, header, "fixture-tool-search", Some("bedrock_mantle"))
}

// tool examples ride along when the provider keeps them, and an added beta always brings them
fn keeps_examples_beta() {
    let both = [TOOL_EXAMPLES_BETA, TOOL_SEARCH_BETA];
    assert_eq!(mantle(SEARCH, ADVANCED), both);
    assert_eq!(mantle("[]", SEARCH_BETA), both);
}

// input examples or programmatic calls never change the added beta
fn keeps_added_examples() {
    assert_eq!(mantle(WITH_EXAMPLES, ADVANCED), [TOOL_SEARCH_BETA]);
    assert_eq!(mantle(PROGRAMMATIC, ADVANCED), [TOOL_SEARCH_BETA]);
}

// the adjust discards the user's advanced beta before the provider filter sees it
fn leaves_untouched() {
    assert_eq!(
        betas("[]", ADVANCED, "fixture-tool-search", Some("anthropic")),
        ["advanced-tool-use-2025-11-20"]
    );
    assert!(betas(SEARCH, ADVANCED, "fixture-tool-search", Some("anthropic")).is_empty());
}

pub(crate) fn run() {
    replaces_advanced_beta();
    keeps_examples_beta();
    keeps_added_examples();
    leaves_untouched();
}
