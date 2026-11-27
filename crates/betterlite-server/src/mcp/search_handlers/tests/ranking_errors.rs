use async_trait::async_trait;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_semantic::{EmbedError, Embedder, Vector};

use super::super::handle_mcp_proxy_tool;
use super::super::handle_mcp_tool_search;
use super::super::ports::RouterEmbedders;
use super::super::types::{McpRequestScope, McpToolSearchRequest, ProxyToolRequest};
use super::{Fake, caller, context, object_arguments, text_of, tool};

struct FailingEmbedder;

#[async_trait]
impl Embedder for FailingEmbedder {
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vector>, EmbedError> {
        Err(EmbedError::Other("embedding down".to_owned()))
    }
}

struct FailingRouter;

impl RouterEmbedders for FailingRouter {
    fn embedder(&self, _embedding_model: &str) -> Box<dyn Embedder> {
        Box::new(FailingEmbedder)
    }
}

fn failing_search_fake() -> Fake {
    let mut fake = Fake::default();
    fake.router = Some(Box::new(FailingRouter));
    fake.tools = vec![tool("alpha-tool", "finds alpha", "{}", "srv")];
    fake.settings.embedding_model = Some("embed".to_owned());
    fake
}

#[tokio::test]
async fn embedding_failures_become_error_text_in_tool_and_proxy_search() {
    let fake = failing_search_fake();
    let caller = caller();
    let scope = McpRequestScope::default();
    let tool_search = handle_mcp_tool_search(
        McpToolSearchRequest {
            query: "alpha",
            top_k: 3,
        },
        &context(&fake, &caller, &scope),
    )
    .await
    .expect("an embedding failure is a tool error");
    assert!(tool_search.is_error);
    assert_eq!(text_of(&tool_search), "embedding down");
    let arguments = object_arguments(&[("query", JsonValue::String("alpha".to_owned()))]);
    let proxy_search = handle_mcp_proxy_tool(
        ProxyToolRequest {
            name: "search_tools",
            arguments: &arguments,
        },
        &context(&fake, &caller, &scope),
    )
    .await
    .expect("an embedding failure is a tool error");
    assert!(proxy_search.is_error);
    assert_eq!(text_of(&proxy_search), "embedding down");
}
