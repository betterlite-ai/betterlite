use serde::Serialize;

use super::definition::RouteDefinition;
use super::error::RouteLoadError;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Route {
    pub name: String,
    pub description: String,
    pub utterances: Vec<String>,
    pub score_threshold: f64,
}

impl Route {
    pub fn from_definition(
        definition: RouteDefinition,
        global_threshold: f64,
    ) -> Result<Self, RouteLoadError> {
        let score_threshold = definition.similarity_threshold.unwrap_or(global_threshold);
        // a NaN or infinite threshold makes every comparison meaningless, so the route is refused by name
        if !score_threshold.is_finite() {
            return Err(RouteLoadError::NonFiniteThreshold {
                name: definition.route_name,
            });
        }
        Ok(Self {
            name: definition.route_name,
            description: definition.description,
            utterances: definition.utterances,
            score_threshold,
        })
    }
}
