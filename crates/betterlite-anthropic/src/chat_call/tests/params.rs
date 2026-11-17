use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::feature_flags::beta::{MCP_CLIENT_BETA, THINKING_DISPLAY_BETA};
use crate::headers::request::TOOL_SEARCH_BETA;

use super::harness::{Setup, execute};

const CUSTOM_BASE: &str = "https://anthropic-proxy.example/v1/messages";
const CUSTOM_KEY: &str = "custom-proxy-key";

fn object(pairs: &[(&str, &str)]) -> Json {
    let mut fields = IndexMap::new();
    for (name, value) in pairs {
        fields.insert((*name).to_owned(), Json::Text((*value).to_owned()));
    }
    Json::Object(fields)
}

// one tool, one MCP server and one thinking block: each raises its own beta header
fn featured_options() -> IndexMap<String, Json> {
    let tool = object(&[
        ("type", "tool_search_tool_regex_20251119"),
        ("name", "lookup"),
    ]);
    let mcp = object(&[("type", "url"), ("name", "search")]);
    let thinking = object(&[("type", "adaptive"), ("display", "updates")]);
    IndexMap::from([
        ("tools".to_owned(), Json::List(vec![tool])),
        ("mcp_servers".to_owned(), Json::List(vec![mcp])),
        ("thinking".to_owned(), thinking),
    ])
}

#[test]
fn chat_call_header_features() {
    let mut setup = Setup::default();
    setup.optional_params = featured_options();
    let run = execute(setup);
    let beta = run.transforms[0]
        .headers
        .get("anthropic-beta")
        .expect("the detected features reach the configured transform");
    assert!(beta.contains(TOOL_SEARCH_BETA));
    assert!(beta.contains(MCP_CLIENT_BETA));
    assert!(beta.contains(THINKING_DISPLAY_BETA));
}

#[test]
fn chat_call_custom_base_bearer() {
    // a custom base, a key without the sk-ant- prefix and the opt-in send a bearer header instead
    let mut bearer_setup = Setup::default();
    bearer_setup.api_base = CUSTOM_BASE;
    bearer_setup.caller_key = Some(CUSTOM_KEY);
    bearer_setup.litellm_params =
        IndexMap::from([("use_bearer_for_custom_base".to_owned(), Json::Bool(true))]);
    let bearer = execute(bearer_setup);
    assert_eq!(
        bearer.transforms[0]
            .headers
            .get("authorization")
            .map(String::as_str),
        Some("Bearer custom-proxy-key")
    );
    assert!(!bearer.transforms[0].headers.contains_key("x-api-key"));

    // without the opt-in the same base keeps the api-key header
    let mut keyed_setup = Setup::default();
    keyed_setup.api_base = CUSTOM_BASE;
    keyed_setup.caller_key = Some(CUSTOM_KEY);
    let keyed = execute(keyed_setup);
    assert_eq!(
        keyed.transforms[0]
            .headers
            .get("x-api-key")
            .map(String::as_str),
        Some(CUSTOM_KEY)
    );
    assert!(!keyed.transforms[0].headers.contains_key("authorization"));
}
