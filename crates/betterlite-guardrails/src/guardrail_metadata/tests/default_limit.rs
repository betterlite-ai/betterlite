use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use rstest::rstest;

use super::support::{data, header, headers, scan_entry_text};
use crate::guardrail_metadata::record_scan_id;

#[rstest]
#[case::a_short_entry_fits(10, true)]
#[case::an_entry_past_the_limit_is_dropped(2100, false)]
fn guardrail_metadata_cases(#[case] id_length: usize, #[case] metadata_header: bool) {
    let long_id = "s".repeat(id_length);
    let mut request_data = data("{}");
    record_scan_id(
        &mut request_data,
        Some(&long_id),
        Some("pii"),
        "panw_prisma_airs",
        GuardrailEventHooks::PreCall,
    );
    let found = headers(&request_data);
    assert_eq!(
        header(&found, "x-litellm-guardrail-scan-id"),
        Some(long_id.as_str())
    );
    let expected = metadata_header.then(|| format!("[{}]", scan_entry_text(&long_id)));
    assert_eq!(
        header(&found, "x-litellm-guardrail-scan-metadata"),
        expected.as_deref()
    );
}
