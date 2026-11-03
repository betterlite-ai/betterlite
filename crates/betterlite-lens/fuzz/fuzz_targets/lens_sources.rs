#![no_main]

use betterlite_lens::sources::activity_availability::ActivityAvailability;
use betterlite_lens::sources::agent_row::AgentRow;
use betterlite_lens::sources::count_row::CountRow;
use betterlite_lens::sources::execution_row::ExecutionRow;
use betterlite_lens::sources::parse_execution::parse_execution;
use betterlite_lens::sources::part_row::PartRow;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = std::str::from_utf8(bytes) {
        drop(parse_execution(text));
    }
    drop(serde_json::from_slice::<Vec<ActivityAvailability>>(bytes));
    drop(serde_json::from_slice::<Vec<ExecutionRow>>(bytes));
    drop(serde_json::from_slice::<Vec<PartRow>>(bytes));
    drop(serde_json::from_slice::<Vec<CountRow>>(bytes));
    drop(serde_json::from_slice::<Vec<AgentRow>>(bytes));
});
