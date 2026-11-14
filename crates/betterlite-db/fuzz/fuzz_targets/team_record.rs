#![no_main]

use betterlite_db::team_repository::decoder::{
    TeamRecord, decode_locked_roster, decode_team_record,
};
use betterlite_db::team_rows::decoder::parse_data;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(record) = TeamRecord::parse(bytes) {
        let _team = decode_team_record(record.as_ref());
    }
    if let Ok(value) = parse_data(bytes) {
        let _roster = decode_locked_roster(&value);
    }
});
