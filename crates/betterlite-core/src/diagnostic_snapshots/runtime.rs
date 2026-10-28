use betterlite_config::loader::EnvironmentLookup;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiagnosticError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UnsupportedSnapshot {
    pub error: DiagnosticError,
}

impl UnsupportedSnapshot {
    #[must_use]
    pub fn gc_statistics() -> Self {
        let measurement = "GC statistics";
        Self::new(measurement)
    }

    #[must_use]
    pub fn object_type_counts() -> Self {
        let measurement = "object type counts";
        Self::new(measurement)
    }

    #[must_use]
    pub fn uncollectable_objects() -> Self {
        let measurement = "uncollectable objects";
        Self::new(measurement)
    }

    fn new(measurement: &str) -> Self {
        let error_type = "unsupported_runtime".to_owned();
        let code = "unsupported_runtime".to_owned();
        Self {
            error: DiagnosticError {
                error_type,
                code,
                message: format!("{measurement} requires CPython runtime introspection"),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CacheComponent {
    pub item_count: usize,
    pub cache_dict_bytes: u64,
    pub ttl_dict_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CacheMemoryInput {
    pub user_api_key_cache: Result<CacheComponent, String>,
    pub llm_router_cache: Result<CacheComponent, String>,
    pub proxy_logging_cache: Result<CacheComponent, String>,
    pub redis_usage_cache: Result<CacheComponent, String>,
}

impl Default for CacheMemoryInput {
    fn default() -> Self {
        Self {
            user_api_key_cache: Ok(CacheComponent::default()),
            llm_router_cache: Ok(CacheComponent::default()),
            proxy_logging_cache: Ok(CacheComponent::default()),
            redis_usage_cache: Ok(CacheComponent::default()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CacheMemorySnapshot {
    pub user_api_key_cache: Result<CacheComponent, String>,
    pub llm_router_cache: Result<CacheComponent, String>,
    pub proxy_logging_cache: Result<CacheComponent, String>,
    pub redis_usage_cache: Result<CacheComponent, String>,
}

#[must_use]
pub fn cache_memory_snapshot(input: CacheMemoryInput) -> CacheMemorySnapshot {
    let cache_input = input;
    CacheMemorySnapshot::from_input(cache_input)
}

impl CacheMemorySnapshot {
    #[must_use]
    pub fn from_input(input: CacheMemoryInput) -> Self {
        let user_api_key_cache = input.user_api_key_cache;
        let llm_router_cache = input.llm_router_cache;
        let proxy_logging_cache = input.proxy_logging_cache;
        let redis_usage_cache = input.redis_usage_cache;
        Self {
            user_api_key_cache,
            llm_router_cache,
            proxy_logging_cache,
            redis_usage_cache,
        }
    }
}

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

#[must_use]
pub fn router_memory_snapshot(input: RouterMemoryInput) -> RouterMemorySnapshot {
    let router_input = input;
    RouterMemorySnapshot::from_input(router_input)
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

pub fn gc_thresholds<L>(config: &L) -> Option<[u64; 3]>
where
    L: EnvironmentLookup,
{
    let value = config.lookup("PYTHON_GC_THRESHOLD")?;
    let mut parts = value.split(',').map(str::trim).map(str::parse::<u64>);
    let thresholds = [
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    ];
    if parts.next().is_some() {
        return None;
    }
    Some(thresholds)
}
