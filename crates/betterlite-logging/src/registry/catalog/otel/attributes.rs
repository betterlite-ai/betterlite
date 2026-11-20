use std::collections::BTreeMap;

use betterlite_config::sections::otel_settings::{OtelAttributes, Settings};

use crate::otel::legacy_metrics::filter::CallbackSettingsView;
use crate::otel::model::utils::coercion::OtelModelValue;

const INCLUDE_LIST: &str = "include_list";
const EXCLUDE_LIST: &str = "exclude_list";

// LiteLLM's OTELMetricAttributeFilter as the filter's untyped input, holding only the keys that are set.
#[must_use]
pub fn value(attributes: Option<&OtelAttributes>) -> Option<OtelModelValue> {
    let attributes = attributes?;
    let mut mapping = BTreeMap::new();
    if let Some(include) = &attributes.include_list {
        mapping.insert(
            INCLUDE_LIST.to_owned(),
            OtelModelValue::Array(
                include
                    .iter()
                    .cloned()
                    .map(OtelModelValue::String)
                    .collect(),
            ),
        );
    }
    if let Some(exclude) = &attributes.exclude_list {
        mapping.insert(
            EXCLUDE_LIST.to_owned(),
            OtelModelValue::Array(
                exclude
                    .iter()
                    .cloned()
                    .map(OtelModelValue::String)
                    .collect(),
            ),
        );
    }
    Some(OtelModelValue::Object(mapping))
}

// the production CallbackSettingsView: otel.attributes as the legacy metric filter and B110 read it
pub struct AttributeSettings {
    value: Option<OtelModelValue>,
}

impl AttributeSettings {
    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            value: value(settings.attributes.as_ref()),
        }
    }
}

impl CallbackSettingsView for AttributeSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        self.value.clone()
    }
}
