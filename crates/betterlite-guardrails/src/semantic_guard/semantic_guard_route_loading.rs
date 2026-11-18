pub mod builder;
pub mod custom_routes;
pub mod definition;
pub mod error;
pub mod route;
pub mod sources;
pub mod templates;
mod tests;

use betterlite_config::loader::EnvironmentLookup;

use self::definition::RouteDefinition;
use self::error::RouteLoadError;
use self::route::Route;
use self::sources::RouteSources;

// LiteLLM reads DEFAULT_SEMANTIC_GUARD_SIMILARITY_THRESHOLD at import time and falls back to 0.75
const SIMILARITY_THRESHOLD_VARIABLE: &str = "DEFAULT_SEMANTIC_GUARD_SIMILARITY_THRESHOLD";
pub const DEFAULT_SIMILARITY_THRESHOLD: f64 = 0.75;

pub fn default_similarity_threshold(
    environment: &dyn EnvironmentLookup,
) -> Result<f64, RouteLoadError> {
    let Some(text) = environment.lookup(SIMILARITY_THRESHOLD_VARIABLE) else {
        return Ok(DEFAULT_SIMILARITY_THRESHOLD);
    };
    Ok(
        betterlite_env::number::<f64>(SIMILARITY_THRESHOLD_VARIABLE, &text)?
            .unwrap_or(DEFAULT_SIMILARITY_THRESHOLD),
    )
}

pub fn build_routes(sources: &RouteSources) -> Result<Vec<Route>, RouteLoadError> {
    // templates first, then the custom routes file, then the inline routes, with no deduplication
    let mut routes = Vec::new();
    for name in &sources.route_templates {
        let definition = templates::load(name)?;
        routes.push(Route::from_definition(
            definition,
            sources.global_threshold,
        )?);
    }
    if let Some(path) = sources.custom_routes_file.as_deref() {
        for definition in custom_routes::load(path)? {
            routes.push(Route::from_definition(
                definition,
                sources.global_threshold,
            )?);
        }
    }
    for (index, value) in sources.custom_routes.iter().enumerate() {
        let definition = RouteDefinition::from_value(value)
            .map_err(|source| RouteLoadError::InlineDefinition { index, source })?;
        routes.push(Route::from_definition(
            definition,
            sources.global_threshold,
        )?);
    }
    tracing::info!("SemanticGuard: built {} routes", routes.len());
    Ok(routes)
}
