use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::support::{scan_entry_text, scan_entry_value};
use crate::guardrail_metadata::serialize_scan_metadata_header;

const IDS: [&str; 3] = ["a", "b", "c"];

fn header_of(scan_ids: &[&str]) -> String {
    let encoded: Vec<String> = scan_ids.iter().map(|id| scan_entry_text(id)).collect();
    format!("[{}]", encoded.join(","))
}

fn expected_header(fitting: &[&str], slack: i64) -> Option<String> {
    let kept = if slack < 0 {
        fitting.len().checked_sub(1)?
    } else {
        fitting.len()
    };
    if kept == 0 {
        return None;
    }
    Some(header_of(&fitting[..kept]))
}

#[rstest]
#[case::one_entry_exactly_at_the_limit(&["a"], 0)]
#[case::one_entry_one_character_short(&["a"], -1)]
#[case::two_entries_exactly_at_the_limit(&["a", "b"], 0)]
#[case::two_entries_one_character_short(&["a", "b"], -1)]
#[case::nothing_fits(&[], 0)]
fn guardrail_metadata_cases(#[case] fitting: &[&str], #[case] slack: i64) {
    let entries: Vec<GuardrailValue> = IDS.iter().map(|id| scan_entry_value(id)).collect();
    let limit = header_of(fitting).chars().count();
    let max_length = limit
        .checked_add_signed(isize::try_from(slack).unwrap())
        .unwrap();
    assert_eq!(
        serialize_scan_metadata_header(&entries, max_length).unwrap(),
        expected_header(fitting, slack)
    );
}
