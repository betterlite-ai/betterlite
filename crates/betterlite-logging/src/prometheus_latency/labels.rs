use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;

use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;
use crate::prometheus_label_schema::PrometheusLabelSettings;

// LiteLLM reads the label settings from module state; betterlite passes them with the values they apply to
#[derive(Clone, Copy)]
pub struct LabelInputs<'labels> {
    pub values: &'labels UserApiKeyLabelValues,
    pub settings: &'labels PrometheusLabelSettings,
    pub end_user_settings: EndUserCostTrackingSettings,
    pub context: Option<&'labels PrometheusLabelFactoryContext>,
}

impl<'labels> LabelInputs<'labels> {
    #[must_use]
    pub const fn source(&self) -> LabelSource<'labels> {
        match self.context {
            Some(context) => LabelSource::Context(context),
            None => LabelSource::Values {
                settings: self.settings,
                end_user_settings: self.end_user_settings,
            },
        }
    }
}
