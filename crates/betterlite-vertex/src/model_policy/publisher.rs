use crate::model_urls::model_name::get_vertex_base_model_name;
use crate::model_urls::route::VertexModelRoute;

use super::route::{RouteParams, get_vertex_ai_model_route};

#[must_use]
pub fn is_self_deployed_endpoint(model: &str) -> bool {
    let local_model = model.strip_prefix("vertex_ai/").unwrap_or(model);
    let route = get_vertex_ai_model_route(local_model, &RouteParams::default());
    if route == VertexModelRoute::Gemma {
        return true;
    }
    // a publisher model keeps its "publisher/" segment and sends the id in the body; a deployed endpoint is one segment
    route == VertexModelRoute::ModelGarden && !get_vertex_base_model_name(local_model).contains('/')
}
