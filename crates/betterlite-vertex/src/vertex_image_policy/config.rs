use crate::model_policy::route::{RouteParams, get_vertex_ai_model_route};
use crate::model_urls::route::VertexModelRoute;

// get_vertex_ai_image_generation_config: the Gemini package only for the Gemini route, Imagen for every other one
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationConfig {
    Gemini,
    Imagen,
}

// get_vertex_ai_image_edit_config asks the same classifier but chooses between the edit packages
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditConfig {
    Gemini,
    Imagen,
}

#[must_use]
pub fn generation_config(model: &str) -> GenerationConfig {
    if get_vertex_ai_model_route(model, &RouteParams::default()) == VertexModelRoute::Gemini {
        GenerationConfig::Gemini
    } else {
        GenerationConfig::Imagen
    }
}

#[must_use]
pub fn edit_config(model: &str) -> EditConfig {
    if get_vertex_ai_model_route(model, &RouteParams::default()) == VertexModelRoute::Gemini {
        EditConfig::Gemini
    } else {
        EditConfig::Imagen
    }
}
