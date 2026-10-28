use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SizedSection {
    pub item_count: usize,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RouterMemoryInput {
    pub model_list: Option<Vec<u8>>,
    pub model_names: Option<BTreeMap<String, usize>>,
    pub deployment_names: Option<Vec<String>>,
    pub deployment_latency_map: Option<BTreeMap<String, f64>>,
    pub fallbacks: Option<Vec<String>>,
    pub router_object_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RouterMemorySnapshot {
    pub model_list: Option<SizedSection>,
    pub model_names: Option<SizedSection>,
    pub deployment_names: Option<SizedSection>,
    pub deployment_latency_map: Option<SizedSection>,
    pub fallbacks: Option<SizedSection>,
    pub router_object: Option<SizedSection>,
    pub error: Option<String>,
}

impl RouterMemorySnapshot {
    #[must_use]
    pub fn from_input(input: RouterMemoryInput) -> Self {
        let router_object_bytes = input.router_object_bytes;
        Self {
            model_list: input.model_list.map(|values| SizedSection {
                item_count: values.len(),
                size_bytes: u64::try_from(values.len()).unwrap_or(u64::MAX),
            }),
            model_names: input.model_names.map(|values| SizedSection {
                item_count: values.len(),
                size_bytes: u64::try_from(values.len()).unwrap_or(u64::MAX),
            }),
            deployment_names: input.deployment_names.map(|values| SizedSection {
                item_count: values.len(),
                size_bytes: u64::try_from(values.len()).unwrap_or(u64::MAX),
            }),
            deployment_latency_map: input.deployment_latency_map.map(|values| SizedSection {
                item_count: values.len(),
                size_bytes: u64::try_from(values.len()).unwrap_or(u64::MAX),
            }),
            fallbacks: input.fallbacks.map(|values| SizedSection {
                item_count: values.len(),
                size_bytes: u64::try_from(values.len()).unwrap_or(u64::MAX),
            }),
            router_object: Some(SizedSection {
                item_count: 1,
                size_bytes: router_object_bytes,
            }),
            error: None,
        }
    }
}
