use betterlite_models::api_base_lookup::ApiBaseError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub trait ApiBaseResolver: Send + Sync {
    fn resolve(
        &self,
        model: &str,
        params: &IndexMap<String, Json>,
    ) -> Result<Option<String>, ApiBaseError>;
}
