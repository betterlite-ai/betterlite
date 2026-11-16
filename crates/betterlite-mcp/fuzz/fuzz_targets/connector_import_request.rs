#![no_main]

use std::collections::BTreeSet;

use betterlite_mcp::connector_import::convert::convert_connector_entries;
use betterlite_mcp::connector_import::request::ConnectorImportRequest;
use betterlite_mcp::server_payload::rules::StdioPolicy;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(request) = ConnectorImportRequest::from_slice(bytes) else {
        return;
    };
    let policy = StdioPolicy {
        enabled: true,
        allowed_commands: BTreeSet::new(),
    };
    let _outcomes = convert_connector_entries(&request, &policy);
});
