use crate::bedrock_messages_beta::tests::support::obj;
use crate::bedrock_messages_beta::web_search::{WebSearchError, reject_web_search_tools};

const MODEL: &str = "claude-opus-4-5";

const MESSAGE: &str = "Bedrock does not support Anthropic's server-side web search tool (tool type 'web_search_20250305', model 'claude-opus-4-5'). To use web search with this model, enable LiteLLM's web search interception so the proxy executes the search instead: https://docs.litellm.ai/docs/integrations/websearch_interception. Alternatively, remove the web_search tool from the request.";

fn reject(tools: &str) -> Result<(), WebSearchError> {
    reject_web_search_tools(&obj(tools), MODEL)
}

pub(crate) fn run() {
    reject(r#"{"max_tokens": 8}"#).expect("a request without tools passes");
    reject(r#"{"tools": "web_search_20250305"}"#).expect("plain text tools pass");
    reject(r#"{"tools": {"type": "web_search_20250305"}}"#).expect("an object tools value passes");

    // a native tool is rejected with the tool type and the requested model
    let error = reject(r#"{"tools": [{"type": "web_search_20250305", "name": "web_search"}]}"#)
        .expect_err("the native web search tool is rejected");
    assert_eq!(
        error,
        WebSearchError::ServerSide {
            tool_type: "web_search_20250305".to_owned(),
            model: MODEL.to_owned(),
        }
    );
    assert_eq!(error.status_code(), 400);
    assert_eq!(error.to_string(), MESSAGE);

    // only the first matching tool names the error
    let first =
        reject(r#"{"tools": [{"type": "web_search_20260201"}, {"type": "web_search_20250305"}]}"#)
            .expect_err("the first native tool names the error");
    assert_eq!(
        first,
        WebSearchError::ServerSide {
            tool_type: "web_search_20260201".to_owned(),
            model: MODEL.to_owned(),
        }
    );

    // converted function tools, bare names, non-dict entries and non-string types pass
    reject(
        r#"{"tools": [{"type": "function", "function": {"name": "web_search"}}, {"name": "web_search"}, "x", {"type": 5}]}"#,
    )
    .expect("converted function tools and non-dict entries pass");
}
