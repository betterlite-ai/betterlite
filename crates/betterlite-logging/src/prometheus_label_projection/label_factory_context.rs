use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_prometheus::label_sanitization::{sanitize_label_name, sanitize_label_value};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;

use super::end_user_caching::CachedEndUser;
use super::end_user_resolution::{CostTrackingEndUserResolver, EndUserResolver};
use super::label_dump_values::sanitized_scalar;
use super::tag_labels::custom_labels_from_tags;
use super::tag_wildcard_matching::TagPatternError;
use crate::end_user::{EndUserCostTrackingSettings, EndUserIdError};
use crate::prometheus_label_schema::PrometheusLabelSettings;

pub struct PrometheusLabelFactoryContext {
    pub(crate) enum_values: UserApiKeyLabelValues,
    pub(crate) sanitized_enum: BTreeMap<String, Option<String>>,
    pub(crate) custom_by_sanitized_key: BTreeMap<String, Option<String>>,
    pub(crate) tag_labels: BTreeMap<String, Option<String>>,
    pub(crate) resolver: Arc<dyn EndUserResolver>,
    pub(crate) resolved_end_user: Mutex<CachedEndUser>,
}

impl PrometheusLabelFactoryContext {
    pub fn new(
        values: &UserApiKeyLabelValues,
        settings: &PrometheusLabelSettings,
        end_user_settings: EndUserCostTrackingSettings,
    ) -> Result<Self, TagPatternError> {
        Self::with_resolver(
            values,
            settings,
            Arc::new(CostTrackingEndUserResolver {
                settings: end_user_settings,
            }),
        )
    }

    pub fn with_resolver(
        values: &UserApiKeyLabelValues,
        settings: &PrometheusLabelSettings,
        resolver: Arc<dyn EndUserResolver>,
    ) -> Result<Self, TagPatternError> {
        let mut sanitized_enum = BTreeMap::new();
        for (key, value) in values.model_dump() {
            sanitized_enum.insert(key, sanitized_scalar(&value));
        }
        let mut custom_by_sanitized_key = BTreeMap::new();
        for (key, value) in &values.custom_metadata_labels {
            custom_by_sanitized_key.insert(
                sanitize_label_name(key),
                sanitize_label_value(Some(value.as_str())),
            );
        }
        let mut tag_labels = BTreeMap::new();
        for (key, value) in custom_labels_from_tags(&values.tags, &settings.custom_tags)? {
            tag_labels.insert(key, sanitize_label_value(Some(value.as_str())));
        }
        Ok(Self {
            enum_values: values.clone(),
            sanitized_enum,
            custom_by_sanitized_key,
            tag_labels,
            resolver,
            resolved_end_user: Mutex::new(CachedEndUser::NotComputed),
        })
    }

    pub fn get_resolved_end_user(&self) -> Result<Option<String>, EndUserIdError> {
        let cached = self
            .resolved_end_user
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match cached {
            CachedEndUser::Computed(resolved) => Ok(resolved),
            CachedEndUser::NotComputed => {
                let resolved = self
                    .resolver
                    .resolve_end_user(self.enum_values.end_user.as_deref())?;
                let mut slot = self
                    .resolved_end_user
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                *slot = CachedEndUser::Computed(resolved.clone());
                drop(slot);
                Ok(resolved)
            }
        }
    }
}

#[path = "label_factory_context/tests.rs"]
mod tests;
