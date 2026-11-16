use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_models::catalog::ModelCatalog;

#[derive(Clone, Copy, Debug)]
pub struct LiveSetupSettings<'catalog> {
    pub catalog: &'catalog ModelCatalog,
    pub schema: &'catalog SchemaSettings,
    pub defer_setup: bool,
}
