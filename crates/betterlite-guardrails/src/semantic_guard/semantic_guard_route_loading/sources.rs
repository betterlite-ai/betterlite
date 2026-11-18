use std::path::PathBuf;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

pub struct RouteSources {
    pub route_templates: Vec<String>,
    pub custom_routes_file: Option<PathBuf>,
    pub custom_routes: Vec<GuardrailValue>,
    pub global_threshold: f64,
}
