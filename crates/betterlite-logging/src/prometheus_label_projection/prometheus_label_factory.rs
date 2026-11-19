use std::collections::BTreeMap;

use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName;
use betterlite_prometheus::label_sanitization::{sanitize_label_name, sanitize_label_value};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;

use super::end_user_resolution::{CostTrackingEndUserResolver, EndUserResolver as _};
use super::label_dump_values::sanitized_scalar;
use super::label_factory_context::PrometheusLabelFactoryContext;
use super::tag_labels::custom_labels_from_tags;
use super::tag_wildcard_matching::TagPatternError;
use crate::end_user::{EndUserCostTrackingSettings, EndUserIdError};
use crate::prometheus_label_schema::PrometheusLabelSettings;

pub type ProjectedLabels = BTreeMap<String, Option<String>>;

const END_USER_LABEL: &str = UserApiKeyLabelName::EndUser.as_str();

// LiteLLM takes the settings it needs from module state; betterlite passes them with the source that reads them
#[derive(Clone, Copy)]
pub enum LabelSource<'labels> {
    Context(&'labels PrometheusLabelFactoryContext),
    Values {
        settings: &'labels PrometheusLabelSettings,
        end_user_settings: EndUserCostTrackingSettings,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum LabelFactoryError {
    #[error("the label context was built from different enum values")]
    ContextMismatch,
    #[error(transparent)]
    EndUser(#[from] EndUserIdError),
    #[error(transparent)]
    TagPattern(#[from] TagPatternError),
}

pub fn prometheus_label_factory(
    supported_enum_labels: &[String],
    enum_values: &UserApiKeyLabelValues,
    source: LabelSource<'_>,
) -> Result<ProjectedLabels, LabelFactoryError> {
    match source {
        LabelSource::Context(context) => {
            labels_from_context(supported_enum_labels, enum_values, context)
        }
        LabelSource::Values {
            settings,
            end_user_settings,
        } => labels_from_values(
            supported_enum_labels,
            enum_values,
            settings,
            end_user_settings,
        ),
    }
}

fn merge_supported(
    labels: &mut ProjectedLabels,
    extra: &BTreeMap<String, Option<String>>,
    supported_enum_labels: &[String],
) {
    for (key, value) in extra {
        if supported_enum_labels.contains(key) {
            labels.insert(key.clone(), value.clone());
        }
    }
}

fn fill_missing(labels: &mut ProjectedLabels, supported_enum_labels: &[String]) {
    for label in supported_enum_labels {
        labels.entry(label.clone()).or_insert(None);
    }
}

// python compares the object identity; betterlite has no identity to compare, so an equal copy passes (docs/compatibility.md)
fn labels_from_context(
    supported_enum_labels: &[String],
    enum_values: &UserApiKeyLabelValues,
    context: &PrometheusLabelFactoryContext,
) -> Result<ProjectedLabels, LabelFactoryError> {
    if context.enum_values != *enum_values {
        return Err(LabelFactoryError::ContextMismatch);
    }
    let mut labels = ProjectedLabels::new();
    for label in supported_enum_labels {
        if let Some(value) = context.sanitized_enum.get(label) {
            labels.insert(label.clone(), value.clone());
        }
    }
    if let Some(end_user) = labels.get_mut(END_USER_LABEL) {
        *end_user = context.get_resolved_end_user()?;
    }
    merge_supported(
        &mut labels,
        &context.custom_by_sanitized_key,
        supported_enum_labels,
    );
    merge_supported(&mut labels, &context.tag_labels, supported_enum_labels);
    fill_missing(&mut labels, supported_enum_labels);
    Ok(labels)
}

fn labels_from_values(
    supported_enum_labels: &[String],
    enum_values: &UserApiKeyLabelValues,
    settings: &PrometheusLabelSettings,
    end_user_settings: EndUserCostTrackingSettings,
) -> Result<ProjectedLabels, LabelFactoryError> {
    let dump = enum_values.model_dump();
    let mut labels = ProjectedLabels::new();
    for (label, value) in &dump {
        if supported_enum_labels.contains(label) {
            labels.insert(label.clone(), sanitized_scalar(value));
        }
    }
    if let Some(end_user) = labels.get_mut(END_USER_LABEL) {
        let resolver = CostTrackingEndUserResolver {
            settings: end_user_settings,
        };
        *end_user = resolver.resolve_end_user(enum_values.end_user.as_deref())?;
    }
    for (key, value) in &enum_values.custom_metadata_labels {
        let sanitized_key = sanitize_label_name(key);
        if supported_enum_labels.contains(&sanitized_key) {
            labels.insert(sanitized_key, sanitize_label_value(Some(value.as_str())));
        }
    }
    for (key, value) in custom_labels_from_tags(&enum_values.tags, &settings.custom_tags)? {
        if supported_enum_labels.contains(&key) {
            labels.insert(key, sanitize_label_value(Some(value.as_str())));
        }
    }
    fill_missing(&mut labels, supported_enum_labels);
    Ok(labels)
}

#[path = "prometheus_label_factory/tests.rs"]
mod tests;
