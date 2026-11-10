#![cfg(test)]

use super::support_tests::control;
use crate::envelope::limits::{async_max_age, sync_max_age, write_ttl};

#[test]
fn sync_max_age_reads_s_maxage_first() {
    assert_eq!(sync_max_age(&control(None, Some(10), Some(20))), Some(10));
}

#[test]
fn async_max_age_reads_s_max_age_first() {
    assert_eq!(async_max_age(&control(None, Some(10), Some(20))), Some(20));
}

#[test]
fn sync_max_age_falls_back_to_s_max_age() {
    assert_eq!(sync_max_age(&control(None, None, Some(20))), Some(20));
}

#[test]
fn async_max_age_falls_back_to_s_maxage() {
    assert_eq!(async_max_age(&control(None, Some(10), None)), Some(10));
}

#[test]
fn absent_max_age_is_unbounded() {
    assert_eq!(sync_max_age(&control(None, None, None)), None);
    assert_eq!(async_max_age(&control(None, None, None)), None);
}

#[test]
fn explicit_zero_s_maxage_is_kept() {
    assert_eq!(sync_max_age(&control(None, Some(0), Some(20))), Some(0));
    assert_eq!(async_max_age(&control(None, Some(0), None)), Some(0));
}

#[test]
fn explicit_zero_s_max_age_is_kept() {
    assert_eq!(async_max_age(&control(None, Some(20), Some(0))), Some(0));
    assert_eq!(sync_max_age(&control(None, None, Some(0))), Some(0));
}

#[test]
fn control_ttl_overrides_the_default() {
    assert_eq!(
        write_ttl(Some(600), &control(Some(30), None, None)),
        Some(30)
    );
}

#[test]
fn default_ttl_applies_without_a_control() {
    assert_eq!(write_ttl(Some(600), &control(None, None, None)), Some(600));
}

#[test]
fn absent_ttl_stays_absent() {
    assert_eq!(write_ttl(None, &control(None, None, None)), None);
}

#[test]
fn explicit_zero_control_ttl_is_kept() {
    assert_eq!(write_ttl(Some(600), &control(Some(0), None, None)), Some(0));
}
