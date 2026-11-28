#![cfg(test)]

use super::super::body::decode_config_update;

const FULL: &[u8] = include_bytes!("../../../fuzz/corpus/config_update/full-config");
const NULL_ROUTER: &[u8] = include_bytes!("../../../fuzz/corpus/config_update/null-router");
const BAD_WEIGHTS: &[u8] = include_bytes!("../../../fuzz/corpus/config_update/bad-weights");
const NOT_AN_OBJECT: &[u8] = include_bytes!("../../../fuzz/corpus/config_update/not-an-object");

#[test]
fn replays_the_config_update_corpus() {
    let decoded = decode_config_update(FULL).unwrap();
    assert!(decoded.router_settings.is_some());
    assert!(decoded.raw_router_settings.is_some());
    let empty = decode_config_update(NULL_ROUTER).unwrap();
    assert!(empty.router_settings.is_none());
    assert!(empty.general_settings.is_none());
    drop(decode_config_update(BAD_WEIGHTS).unwrap_err());
    drop(decode_config_update(NOT_AN_OBJECT).unwrap_err());
}
