#![cfg(test)]

use std::sync::Arc;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_context::local_ttl_cache::CacheValue;

use super::*;
use crate::cache_backends::convert;

const ITEM_KIB: i64 = 1024;

fn native_memory() -> NativeMemory {
    Python::initialize();
    let clock = Arc::new(convert::wall_clock);
    let ops = LocalMemoryOps::new(ITEM_KIB, clock).unwrap();
    NativeMemory {
        ops: Arc::new(ops),
        default_ttl: None,
    }
}

#[test]
fn backends_memory_round_trip() {
    let native = native_memory();
    native.set("key", r#"{"a":1}"#, None).unwrap();
    assert_eq!(native.get("key").unwrap(), Some(r#"{"a":1}"#.to_owned()));
    assert_eq!(native.get("missing").unwrap(), None);
    native.delete(vec!["key".to_owned()]).unwrap();
    assert_eq!(native.get("key").unwrap(), None);
}

#[test]
fn backends_memory_counts_increments() {
    let native = native_memory();
    assert_eq!(native.increment("count", 1.0_f64, None).unwrap(), "1.0");
    assert_eq!(native.increment("count", 2.0_f64, None).unwrap(), "3.0");
}

#[test]
fn backends_memory_ttl_reads_the_expiry() {
    let native = native_memory();
    native.set("key", "1", Some(60.0_f64)).unwrap();
    let expiry = native.ttl("key").unwrap().unwrap().parse::<f64>().unwrap();
    assert!(expiry > 1_700_000_000.0_f64);
    assert_eq!(native.ttl("missing").unwrap(), None);
}

#[test]
fn backends_memory_flush_and_ping() {
    let native = native_memory();
    native.set("key", "1", None).unwrap();
    native.flush().unwrap();
    assert_eq!(native.get("key").unwrap(), None);
    assert!(native.ping().unwrap());
    native.close().unwrap();
}

#[test]
fn backends_memory_set_value_writes() {
    let native = native_memory();
    set_value(&native.ops, "key", CacheValue::Integer(1), None).unwrap();
    assert_eq!(
        answer_text(native.ops.get_cache("key").unwrap()).unwrap(),
        Some("1".to_owned())
    );
}

#[test]
fn backends_memory_answers_and_remaining_ttl() {
    let value = CacheValue::Text("x".to_owned());
    assert_eq!(answer_text(Some(value)).unwrap(), Some("\"x\"".to_owned()));
    assert_eq!(answer_text(None).unwrap(), None);
    assert_eq!(
        remaining_ttl(vec![("key".to_owned(), 2.5_f64)], "key").unwrap(),
        Some("2.5".to_owned())
    );
    assert_eq!(
        remaining_ttl(vec![("key".to_owned(), 2.5_f64)], "other").unwrap(),
        None
    );
}
