use betterlite_anthropic::capabilities::supported_params::get_supported_openai_params;
use serde::Deserialize;
use serde_json::Value;

use super::adapter::{Adapter, Environment};
use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
pub(super) struct SupportedParamsStep {
    model: String,
}

impl SupportedParamsStep {
    pub(super) async fn answer(
        &self,
        adapter: &Adapter<'_>,
        environment: &mut Environment<'_>,
    ) -> Result<Value, Box<HelperError>> {
        let params = get_supported_openai_params(
            &self.model,
            Some("anthropic"),
            environment,
            &adapter.catalogs,
        )
        .await;
        serde_json::to_value(params).or_value_error()
    }
}
