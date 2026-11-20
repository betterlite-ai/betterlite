#![cfg(test)]

use std::sync::{Mutex, PoisonError};

use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};

use super::end_user_resolution::EndUserResolver;
use crate::end_user::{EndUserCostTrackingSettings, EndUserIdError};
use crate::prometheus_label_schema::PrometheusLabelSettings;
use crate::spend_metadata::SpendMetadataError;

pub(crate) fn values(entries: &[(&str, &str)]) -> UserApiKeyLabelValues {
    let mapped: Vec<(String, LabelDumpValue)> = entries
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_owned(),
                LabelDumpValue::String((*value).to_owned()),
            )
        })
        .collect();
    UserApiKeyLabelValues::from_entries(&mapped)
}

pub(crate) fn texts(items: &[&str]) -> Vec<String> {
    let mut owned = Vec::with_capacity(items.len());
    for item in items {
        owned.push((*item).to_owned());
    }
    owned
}

pub(crate) fn settings(custom_tags: &[&str]) -> PrometheusLabelSettings {
    let mut settings = PrometheusLabelSettings::with_defaults();
    settings.custom_tags = texts(custom_tags);
    settings
}

pub(crate) const PROMETHEUS_ONLY: EndUserCostTrackingSettings = EndUserCostTrackingSettings {
    disable_end_user_cost_tracking: None,
    enable_prometheus_only: Some(true),
};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Resolution {
    Echo,
    Missing,
    Failure,
}

pub(crate) struct CountingResolver {
    pub(crate) resolution: Resolution,
    pub(crate) calls: Mutex<usize>,
}

impl CountingResolver {
    pub(crate) fn new(resolution: Resolution) -> Self {
        Self {
            resolution,
            calls: Mutex::new(0),
        }
    }
}

impl EndUserResolver for CountingResolver {
    fn resolve_end_user(&self, end_user: Option<&str>) -> Result<Option<String>, EndUserIdError> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        let count = calls.saturating_add(1);
        *calls = count;
        drop(calls);
        match self.resolution {
            Resolution::Echo => Ok(end_user.map(|id| format!("resolved-{id}"))),
            Resolution::Missing => Ok(None),
            Resolution::Failure => Err(EndUserIdError::Metadata(SpendMetadataError::FieldType {
                field: "user_api_key_end_user_id",
            })),
        }
    }
}
