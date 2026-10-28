#![cfg(test)]

use super::cache::{CacheComponent, CacheMemoryInput, CacheMemorySnapshot, CacheSectionError};
use super::gc_config::gc_thresholds;
use super::process::{
    MemoryStatus, ProcessMemoryError, health_status, parse_proc_memory, process_memory_snapshot,
    snapshot_process_memory,
};
use super::router::{RouterMemoryInput, RouterMemorySnapshot};
use super::unsupported::UnsupportedSnapshot;
use betterlite_config::loader::ConfigLoader;
use rstest::rstest;
use std::collections::BTreeMap;
use std::error::Error as _;
use std::fs;
use std::io;
use std::path::Path;

#[test]
fn process_thresholds_match_litellm_strict_boundaries() {
    assert_eq!(health_status(80.0_f64), MemoryStatus::Warning);
    assert_eq!(health_status(80.01_f64), MemoryStatus::Critical);
    assert_eq!(health_status(60.0_f64), MemoryStatus::Healthy);
    assert_eq!(health_status(60.01_f64), MemoryStatus::Warning);
}

#[test]
fn process_snapshot_reports_proc_values() {
    let proc_memory = parse_proc_memory(b"100 25 0\n", b"MemFree: 1 kB\nMemTotal: 200 kB\n", 4096)
        .expect("valid proc sample");
    let snapshot = process_memory_snapshot(Ok(proc_memory));
    let snapshot_memory = snapshot.memory.expect("successful process snapshot");
    assert_eq!(snapshot_memory.resident_bytes, 102_400);
    assert_eq!(snapshot_memory.virtual_bytes, 409_600);
    assert!((snapshot_memory.percent - 50.0_f64).abs() < f64::EPSILON);
    assert_eq!(snapshot.status, MemoryStatus::Healthy);
    assert!(snapshot.error.is_none());
}

#[test]
fn process_snapshot_keeps_healthy_status_on_error() {
    let snapshot = process_memory_snapshot(Err(ProcessMemoryError::Unsupported));
    assert_eq!(snapshot.status, MemoryStatus::Healthy);
    assert!(snapshot.error.is_some());
}

#[cfg(target_os = "linux")]
#[test]
fn live_process_snapshot_has_real_memory_values() {
    let snapshot = snapshot_process_memory();
    let memory = snapshot.memory.expect("Linux proc memory is available");
    assert!(memory.resident_bytes > 0);
    assert!(memory.virtual_bytes >= memory.resident_bytes);
    assert!(memory.percent.is_finite());
}

#[test]
fn cache_sections_are_independent() {
    let input = CacheMemoryInput {
        user_api_key_cache: Err(CacheSectionError::Unavailable {
            source: io::Error::other("cache unavailable"),
        }),
        llm_router_cache: Ok(CacheComponent {
            item_count: 2,
            cache_dict_bytes: 10,
            ttl_dict_bytes: 5,
        }),
        proxy_logging_cache: Ok(CacheComponent::default()),
        redis_usage_cache: Ok(CacheComponent::default()),
    };
    let snapshot = CacheMemorySnapshot::from_input(input);
    let cache_error = snapshot.user_api_key_cache.unwrap_err();
    assert_eq!(
        cache_error.to_string(),
        "cache section unavailable: cache unavailable"
    );
    assert!(cache_error.source().is_some());
    assert_eq!(snapshot.llm_router_cache.unwrap().item_count, 2);
}

#[test]
fn router_sections_measure_shallow_inputs() {
    let input = RouterMemoryInput {
        model_list: Some(vec![1, 2]),
        model_names: Some(BTreeMap::from([(String::from("a"), 1)])),
        deployment_names: Some(vec![String::from("a")]),
        deployment_latency_map: Some(BTreeMap::from([(String::from("a"), 1.0_f64)])),
        fallbacks: Some(vec![String::from("fallback")]),
        router_object_bytes: 128,
    };
    let snapshot = RouterMemorySnapshot::from_input(input);
    assert_eq!(snapshot.model_list.unwrap().item_count, 2);
    assert_eq!(snapshot.router_object.unwrap().size_bytes, 128);
}

#[test]
fn python_runtime_snapshots_are_explicitly_unsupported() {
    let snapshot = UnsupportedSnapshot::gc_statistics();
    assert_eq!(snapshot.error.error_type, "unsupported_runtime");
    assert_eq!(snapshot.error.code, "unsupported_runtime");
}

#[test]
fn gc_threshold_uses_config_lookup_boundary() {
    let values = BTreeMap::from([(
        String::from("PYTHON_GC_THRESHOLD"),
        String::from("700,10,10"),
    )]);
    let config = ConfigLoader { lookup: values };
    assert_eq!(gc_thresholds(&config.lookup), Some([700, 10, 10]));
}

#[rstest]
#[case("valid")]
fn replays_the_proc_parser_corpus(#[case] corpus_name: &str) {
    let path = Path::new("fuzz/corpus/proc_statm_meminfo").join(corpus_name);
    let bytes = fs::read(path).expect("corpus file is readable");
    let split = bytes.len().checked_div(2).expect("division by nonzero");
    let _unused = parse_proc_memory(&bytes[..split], &bytes[split..], 4096);
}
