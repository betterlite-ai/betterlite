use std::time::Instant;

use betterlite_models::call_helpers::metadata::MetadataSlots;
use betterlite_models::model_registration::JsonObject;
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus::failure::RequestedModelCatalog;
use crate::prometheus::failure::exception::LabelException;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;

#[derive(Clone, Copy)]
pub struct DeploymentLabels<'labels> {
    pub litellm_model_name: &'labels str,
    pub model_id: Option<&'labels str>,
    pub api_base: Option<&'labels str>,
    pub api_provider: &'labels str,
}

#[derive(Clone, Copy)]
pub struct CooledDownLabels<'labels> {
    pub litellm_model_name: &'labels str,
    pub model_id: &'labels str,
    pub api_base: &'labels str,
    pub api_provider: &'labels str,
    pub exception_status: &'labels str,
}

#[derive(Clone, Copy)]
pub struct BatchCreatedLabels<'labels> {
    pub model: Option<&'labels str>,
    pub api_provider: Option<&'labels str>,
    pub user: Option<&'labels str>,
    pub user_email: Option<&'labels str>,
    pub api_key_alias: Option<&'labels str>,
}

// kwargs["tags"], which LiteLLM declares as a list of strings
pub struct FallbackRequest<'request> {
    pub original_model_group: &'request str,
    pub model: Option<&'request str>,
    pub tags: &'request [String],
    pub metadata: &'request MetadataSlots,
    pub exception: &'request LabelException,
    pub catalog: Option<&'request dyn RequestedModelCatalog>,
    pub now: Instant,
}

pub struct FailureRequest<'request> {
    pub model: Option<&'request str>,
    pub exception: Option<&'request LabelException>,
    pub litellm_params: &'request JsonObject,
    pub payload: Option<&'request StandardLoggingPayload>,
    pub catalog: Option<&'request dyn RequestedModelCatalog>,
    pub now: Instant,
}

pub struct SuccessRequest<'request> {
    pub model: Option<&'request str>,
    pub stream: Option<bool>,
    pub completion_start_time: Option<Decimal>,
    pub litellm_params: &'request JsonObject,
    pub payload: Option<&'request StandardLoggingPayload>,
    pub exception: Option<&'request LabelException>,
    pub values: &'request UserApiKeyLabelValues,
    pub context: Option<&'request PrometheusLabelFactoryContext>,
    pub start_time: Decimal,
    pub end_time: Decimal,
    pub output_tokens: Option<Decimal>,
    pub now: Instant,
}
