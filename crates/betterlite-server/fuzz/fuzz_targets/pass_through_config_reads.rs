#![no_main]

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_server::endpoints::passthrough_config_reads::config_read::config_endpoints;
use betterlite_server::endpoints::passthrough_config_reads::ports::ConfigEndpointsSource;
use betterlite_server::endpoints::passthrough_config_reads::record::PassThroughGenericEndpoint;
use libfuzzer_sys::fuzz_target;

struct OneEntry {
    values: Vec<JsonData>,
}

impl ConfigEndpointsSource for OneEntry {
    fn configured_endpoints(&self) -> Option<&[JsonData]> {
        Some(&self.values)
    }
}

fuzz_target!(|text: &str| {
    let Ok(value) = serde_json::from_str::<JsonData>(text) else {
        return;
    };
    let _endpoint = PassThroughGenericEndpoint::from_json(&value, true);
    let source = OneEntry {
        values: vec![value],
    };
    let _endpoints = config_endpoints(&source);
});
