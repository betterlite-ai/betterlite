use crate::model_urls::model_name::is_decimal;
use crate::model_urls::route::VertexModelRoute;

use super::partners::is_vertex_partner_model;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RouteParams<'params> {
    pub base_model: Option<&'params str>,
    pub api_base: Option<&'params str>,
}

#[must_use]
pub fn get_vertex_ai_model_route(model: &str, params: &RouteParams<'_>) -> VertexModelRoute {
    if let Some(base_model) = params.base_model
        && base_model.contains("gemini")
    {
        return VertexModelRoute::Gemini;
    }
    if model.contains("agent_engine/") {
        return VertexModelRoute::AgentEngine;
    }
    let numeric_endpoint =
        is_decimal(model) && params.api_base.is_some_and(|api_base| !api_base.is_empty());
    if numeric_endpoint {
        return VertexModelRoute::Gemini;
    }
    if is_vertex_partner_model(model) {
        return VertexModelRoute::PartnerModels;
    }
    if model.to_lowercase().contains("bge") {
        return VertexModelRoute::Bge;
    }
    if model.contains("gemma/") {
        return VertexModelRoute::Gemma;
    }
    if model.contains("openai") || model.starts_with("xai/") {
        return VertexModelRoute::ModelGarden;
    }
    if model.contains("gemini") {
        return VertexModelRoute::Gemini;
    }
    VertexModelRoute::NonGemini
}
