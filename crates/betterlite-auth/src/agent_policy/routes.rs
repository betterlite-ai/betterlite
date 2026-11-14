use crate::auxiliary_routes;
use crate::route_match::templates::check_route_access;

pub const MANAGED_AGENT_ROUTES: &[&str] = &["/agents", "/v1/agents"];

pub const MANAGED_REALTIME_ROUTES: &[&str] = &["/realtime", "/v1/realtime", "/openai/v1/realtime"];

pub const MANAGED_MODEL_ROUTES: &[&str] = &[
    "/chat/completions",
    "/v1/chat/completions",
    "/completions",
    "/v1/completions",
    "/embeddings",
    "/v1/embeddings",
    "/responses",
    "/v1/responses",
    "/messages",
    "/v1/messages",
    "/messages/count_tokens",
    "/v1/messages/count_tokens",
    "/images/generations",
    "/v1/images/generations",
    "/images/edits",
    "/v1/images/edits",
    "/audio/transcriptions",
    "/v1/audio/transcriptions",
    "/audio/speech",
    "/v1/audio/speech",
    "/moderations",
    "/v1/moderations",
    "/rerank",
    "/v1/rerank",
    "/decisions",
    "/v1/decisions",
    "/systemone",
    "/v1/systemone",
    "/ocr",
    "/v1/ocr",
    "/openai/v1/responses",
    "/v2/rerank",
    "/claude_code_gateway/v1/messages",
    "/claude_code_gateway/v1/messages/count_tokens",
    "/cursor/chat/completions",
];

pub const MANAGED_MODEL_PATHS: &[&str] = &[
    "/engines/{model:path}/chat/completions",
    "/engines/{model:path}/completions",
    "/engines/{model:path}/embeddings",
    "/openai/deployments/{model:path}/chat/completions",
    "/openai/deployments/{model:path}/completions",
    "/openai/deployments/{model:path}/embeddings",
    "/openai/deployments/{model:path}/images/generations",
    "/openai/deployments/{model:path}/images/edits",
    "/v1beta/models/{model_name:path}:countTokens",
    "/v1beta/models/{model_name:path}:generateContent",
    "/v1beta/models/{model_name:path}:streamGenerateContent",
    "/models/{model_name:path}:countTokens",
    "/models/{model_name:path}:generateContent",
    "/models/{model_name:path}:streamGenerateContent",
];

#[must_use]
pub fn managed_agent_route_allowed(route: &str, method: Option<&str>) -> bool {
    if MANAGED_AGENT_ROUTES.contains(&route) {
        return matches!(method, None | Some("GET" | "HEAD"));
    }
    if MANAGED_REALTIME_ROUTES.contains(&route) {
        return matches!(method, None | Some("GET"));
    }
    if MANAGED_MODEL_ROUTES.contains(&route) || check_route_access(route, MANAGED_MODEL_PATHS) {
        return matches!(method, None | Some("POST"));
    }
    let managed_mcp = route != "/token"
        && route != "/introspect"
        && check_route_access(route, auxiliary_routes::MCP_INFERENCE_ROUTES);
    managed_mcp || check_route_access(route, auxiliary_routes::AGENT_INFERENCE_ROUTES)
}
