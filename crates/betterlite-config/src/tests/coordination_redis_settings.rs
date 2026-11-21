use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::parse_view;
use crate::sections::settings::{JsonValue, Section, SettingsStore};

const FILE_BLOCK: &str = include_str!("../../fuzz/corpus/config_yaml/coordination-redis-file.yaml");
const EMPTY_BLOCK: &str =
    include_str!("../../fuzz/corpus/config_yaml/coordination-redis-empty.yaml");

// The proxy config hands general_settings to its section stores as raw JSON; this view is the file half of that read.
#[derive(Debug, Deserialize)]
struct RawConfig {
    general_settings: Option<BTreeMap<String, JsonValue>>,
}

fn general_settings(source: &str) -> SettingsStore {
    let (raw, _ignored) = parse_view::<RawConfig>(source).expect("the config parses");
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(raw.general_settings.unwrap_or_default());
    store
}

#[test]
fn loads_coordination_redis_settings() {
    let store = general_settings(FILE_BLOCK);
    let block = store
        .get("coordination_redis")
        .expect("the coordination block loads");
    let members = block.as_object().expect("the block is an object");
    assert_eq!(
        members.get("host").and_then(JsonValue::as_str),
        Some("os.environ/REDIS_HOST"),
    );
    assert_eq!(
        members.get("password").and_then(JsonValue::as_str),
        Some("os.environ/REDIS_PASSWORD"),
    );
    assert_eq!(members.get("port").and_then(JsonValue::as_i64), Some(6379),);
    let nodes = members
        .get("startup_nodes")
        .and_then(JsonValue::as_array)
        .expect("the startup nodes load");
    assert_eq!(nodes.len(), 1);
    assert_eq!(
        nodes[0].get("host").and_then(JsonValue::as_str),
        Some("127.0.0.1"),
    );
    // The UI defaults are metadata: an absent ssl is not written into the loaded block.
    assert_eq!(members.get("ssl"), None);
    assert_eq!(members.get("sentinel_password"), None);

    let empty = general_settings(EMPTY_BLOCK);
    let empty_block = empty
        .get("coordination_redis")
        .expect("an empty block is still a block");
    assert_eq!(
        empty_block.as_object().map(|members| members.len()),
        Some(0),
    );
}
