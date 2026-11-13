#![no_main]

use betterlite_vertex::studio_agents::decode::{
    AgentOperation, RawResponse, decode_studio_agent_delete, decode_studio_agent_payload,
};
use indexmap::IndexMap;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let headers = IndexMap::new();
    let response = RawResponse {
        status: 200,
        headers: &headers,
        body: bytes,
    };
    let operations = [
        AgentOperation::Create,
        AgentOperation::Get,
        AgentOperation::List,
        AgentOperation::Versions,
    ];
    for operation in operations {
        drop(decode_studio_agent_payload(&response, operation, "fuzz"));
    }
    drop(decode_studio_agent_delete(&response, "fuzz"));
});
