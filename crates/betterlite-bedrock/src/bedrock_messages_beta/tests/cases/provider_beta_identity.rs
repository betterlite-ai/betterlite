use std::collections::BTreeSet;

use crate::bedrock_messages_beta::provider::{CUSTOM_LLM_PROVIDER, beta_headers_provider};
use crate::bedrock_messages_beta::tests::support::{Fixture, Session, list};

const TOOL_SEARCH: &str = r#"[{"type": "tool_search_tool_regex_20251119", "name": "tool_search"}]"#;
const ADVANCED: &str = "advanced-tool-use-2025-11-20";
const TOOL_SEARCH_BETA: &str = "tool-search-tool-2025-10-19";

pub(super) fn run() {
    assert_eq!(CUSTOM_LLM_PROVIDER, "bedrock");
    assert_eq!(beta_headers_provider(None), "bedrock");
    assert_eq!(beta_headers_provider(Some("")), "bedrock");
    assert_eq!(
        beta_headers_provider(Some("bedrock_mantle")),
        "bedrock_mantle"
    );

    // the capability answer, its catalog fallback and the unmapped miss all pass through
    let fixture = Fixture::new();
    let mut session = Session::new(&fixture);
    let tools = list(TOOL_SEARCH);
    let models = [
        "claude-sonnet-4-5",
        "fixture-tool-search",
        "fixture-no-flag",
        "claude-sonnet-4-5-20990101",
        "mystery-model",
    ];
    for (model, supported) in models.into_iter().zip([false, true, false, true, false]) {
        let mut betas = BTreeSet::from([ADVANCED.to_owned()]);
        session.adjust(model, &tools, &mut betas);
        assert!(
            !betas.contains(ADVANCED),
            "{model} discards the advanced beta"
        );
        assert_eq!(betas.contains(TOOL_SEARCH_BETA), supported, "{model}");
    }
}
