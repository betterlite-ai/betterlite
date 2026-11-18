use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::ptu::charges::deployment::RouterEntry;
use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

// The config side of a rollup scan: B844's deployments cloned once, so a catalog reload never changes a run's inputs.
#[derive(Default)]
pub struct CatalogSnapshot {
    pub entries: Vec<RouterEntry>,
}

impl CatalogSnapshot {
    #[must_use]
    pub fn from_specs(specs: &[DeploymentSpec]) -> Self {
        Self {
            entries: specs.iter().map(entry_of).collect(),
        }
    }
}

fn entry_of(spec: &DeploymentSpec) -> RouterEntry {
    RouterEntry {
        model_name: spec.model_name.clone(),
        model_info: json_data(&Json::Object(spec.model_info.clone())),
    }
}

// DeploymentSpec carries the model_info object; the scan reads it as the column's JsonData, whose object shapes differ only in key order.
fn json_data(value: &Json) -> JsonData {
    match value {
        Json::Null => JsonData::Null,
        Json::Bool(flag) => JsonData::Bool(*flag),
        Json::Number(number) => JsonData::Number(number.clone()),
        Json::Text(text) => JsonData::String(text.clone()),
        Json::List(items) => JsonData::Array(items.iter().map(json_data).collect()),
        Json::Object(fields) => JsonData::Object(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), json_data(field)))
                .collect(),
        ),
    }
}
