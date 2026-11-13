#![no_main]

use betterlite_vertex::vertex_agent_engine::records::decode_vertex_agent_buffer;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| drop(decode_vertex_agent_buffer(200, bytes, "agent_engine/7")));
