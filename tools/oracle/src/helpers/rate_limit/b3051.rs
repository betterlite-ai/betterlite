use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_rate_limit::dynamic_priority::allocation::priority_allocation;
use betterlite_rate_limit::dynamic_priority::cache::SaturationCache;
use betterlite_rate_limit::dynamic_priority::descriptors::{
    model_tracking_descriptor, priority_based_descriptors,
};
use betterlite_rate_limit::dynamic_priority::encoding::is_latin1_encodable;
use betterlite_rate_limit::dynamic_priority::normalization::normalize_priority_weights;
use betterlite_rate_limit::dynamic_priority::resolution::priority_from_key_dict;
use betterlite_rate_limit::dynamic_priority::saturation::check_model_saturation;
use betterlite_rate_limit::dynamic_priority::settings::{PriorityPolicy, PrioritySettings};
use betterlite_rate_limit::limiter_records::CacheCounterValue;
use betterlite_rate_limit::provider_attribution::{ModelGroupLimits, PriorityValue};
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum CounterInput {
    Int(i64),
    Text(String),
}

impl CounterInput {
    fn counter_value(&self) -> CacheCounterValue {
        match self {
            Self::Int(number) => CacheCounterValue::Int(*number),
            Self::Text(text) => CacheCounterValue::Text(text.clone()),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Latin1 {
        values: Vec<String>,
    },
    Allocation {
        model: String,
        #[serde(default)]
        rpm: Option<i64>,
        #[serde(default)]
        tpm: Option<i64>,
        window_size: i64,
        #[serde(default)]
        priority_reservation: Option<BTreeMap<String, PriorityValue>>,
        #[serde(default)]
        team_metadata: Option<BTreeMap<String, JsonData>>,
        #[serde(default)]
        key_metadata: BTreeMap<String, JsonData>,
    },
    Saturation {
        model: String,
        #[serde(default)]
        rpm: Option<i64>,
        #[serde(default)]
        tpm: Option<i64>,
        #[serde(default)]
        cache: BTreeMap<String, CounterInput>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Latin1 { values } => Ok(json!({"encodable": encodable_values(values)})),
            Input::Allocation {
                model,
                rpm,
                tpm,
                window_size,
                priority_reservation,
                team_metadata,
                key_metadata,
            } => Ok(allocation_answer(&AllocationCase {
                model,
                limits: ModelGroupLimits {
                    rpm: *rpm,
                    tpm: *tpm,
                },
                window_size: *window_size,
                reservation: priority_reservation.as_ref(),
                team_metadata: team_metadata.as_ref(),
                key_metadata,
            })),
            Input::Saturation {
                model,
                rpm,
                tpm,
                cache,
            } => saturation_answer(&SaturationCase {
                model,
                limits: ModelGroupLimits {
                    rpm: *rpm,
                    tpm: *tpm,
                },
                cache,
            }),
        }
    }
}

struct AllocationCase<'case> {
    model: &'case String,
    limits: ModelGroupLimits,
    window_size: i64,
    reservation: Option<&'case BTreeMap<String, PriorityValue>>,
    team_metadata: Option<&'case BTreeMap<String, JsonData>>,
    key_metadata: &'case BTreeMap<String, JsonData>,
}

struct SaturationCase<'case> {
    model: &'case String,
    limits: ModelGroupLimits,
    cache: &'case BTreeMap<String, CounterInput>,
}

fn encodable_values(values: &[String]) -> Vec<bool> {
    values
        .iter()
        .map(String::as_str)
        .map(is_latin1_encodable)
        .collect()
}

fn allocation_answer(case: &AllocationCase<'_>) -> serde_json::Value {
    let policy = priority_policy(case.reservation);
    let priority = priority_from_key_dict(case.team_metadata, case.key_metadata);
    let weights = normalize_priority_weights(&policy, Some(&case.limits));
    let allocation =
        priority_allocation(&policy, case.model, priority, &weights, Some(&case.limits));
    let descriptors = priority_based_descriptors(
        &policy,
        case.model,
        priority,
        Some(&case.limits),
        case.window_size,
    );
    let tracking = model_tracking_descriptor(case.model, &case.limits, 1, case.window_size);
    json!({
        "weights": weights,
        "allocation": {"weight": allocation.weight, "key": allocation.key},
        "descriptors": descriptors,
        "tracking": tracking,
    })
}

fn priority_policy(reservation: Option<&BTreeMap<String, PriorityValue>>) -> PriorityPolicy {
    PriorityPolicy {
        settings: PrioritySettings::default(),
        reservation: reservation.cloned(),
    }
}

fn saturation_answer(case: &SaturationCase<'_>) -> HelperResult {
    let cache = RecordingCache::seeded(case.cache);
    let runtime = Builder::new_current_thread().build()?;
    let saturation = runtime.block_on(check_model_saturation(
        case.model,
        &case.limits,
        &cache,
        &PriorityPolicy::default(),
    ));
    Ok(json!({"saturation": saturation, "reads": cache.reads()}))
}

struct RecordingCache {
    values: BTreeMap<String, CacheCounterValue>,
    reads: Mutex<Vec<ReadRecord>>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct ReadRecord {
    key: String,
    local_only: bool,
    ttl: i64,
}

impl RecordingCache {
    fn seeded(entries: &BTreeMap<String, CounterInput>) -> Self {
        let values = entries
            .iter()
            .map(|(key, value)| (key.clone(), value.counter_value()))
            .collect();
        Self {
            values,
            reads: Mutex::new(Vec::new()),
        }
    }

    fn reads(&self) -> Vec<ReadRecord> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl SaturationCache for RecordingCache {
    async fn get_cache(
        &self,
        key: &str,
        local_only: bool,
        local_ttl: i64,
    ) -> Option<CacheCounterValue> {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        reads.push(ReadRecord {
            key: key.to_owned(),
            local_only,
            ttl: local_ttl,
        });
        drop(reads);
        self.values.get(key).cloned()
    }
}
