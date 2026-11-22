#![no_main]

use betterlite_server::spend::tag_reports::query::decode_tag_report_query;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let decoded = decode_tag_report_query(bytes);
    let mut rebuilt: Vec<String> = Vec::new();
    for (name, value) in [
        ("user_id", &decoded.user_id),
        ("start_date", &decoded.start_date),
        ("end_date", &decoded.end_date),
        ("tags", &decoded.tags),
    ] {
        if let Some(stored) = value {
            let encoded: String = form_urlencoded::byte_serialize(stored.as_bytes()).collect();
            rebuilt.push(format!("{name}={encoded}"));
        }
    }
    assert_eq!(
        decode_tag_report_query(rebuilt.join("&").as_bytes()),
        decoded
    );
    match decoded.selected_tags() {
        Some(selected) => assert_eq!(
            selected.join(","),
            decoded.tags.as_deref().unwrap_or_default()
        ),
        None => assert!(decoded.tags.as_deref().is_none_or(str::is_empty)),
    }
});
