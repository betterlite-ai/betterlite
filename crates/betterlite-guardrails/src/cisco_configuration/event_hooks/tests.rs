#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::{env, fs, process};

use betterlite_config::sections::cisco_verdict_contracts::InspectionType;
use rstest::rstest;
use tracing::subscriber::set_default;

use crate::cisco_configuration::event_hooks::{
    EventHook, EventHookInput, SUPPORTED_EVENT_HOOKS, infer_inspection_from_mode, is_mcp_call_type,
    normalize_event_hooks, surface_matches, warn_if_mode_surface_mismatch,
};

#[rstest]
#[case::mcp_call(Some("mcp_call"), true)]
#[case::call_mcp_tool(Some("call_mcp_tool"), true)]
#[case::completion(Some("completion"), false)]
#[case::missing(None, false)]
fn is_mcp_call_type_matches_the_mcp_call_types(
    #[case] call_type: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(is_mcp_call_type(call_type), expected);
}

#[rstest]
#[case::mcp_configured_mcp_traffic(InspectionType::Mcp, true, true)]
#[case::mcp_configured_chat_traffic(InspectionType::Mcp, false, false)]
#[case::chat_configured_chat_traffic(InspectionType::Chat, false, true)]
#[case::chat_configured_mcp_traffic(InspectionType::Chat, true, false)]
fn surface_matches_checks_inspection_type_against_traffic(
    #[case] inspection_type: InspectionType,
    #[case] is_mcp_traffic: bool,
    #[case] expected: bool,
) {
    assert_eq!(surface_matches(inspection_type, is_mcp_traffic), expected);
}

#[test]
fn normalize_event_hooks_returns_empty_set_when_absent() {
    assert_eq!(normalize_event_hooks(None), BTreeSet::new());
}

#[test]
fn normalize_event_hooks_coerces_a_single_hook() {
    let input = EventHookInput::Single("pre_call".to_owned());
    assert_eq!(
        normalize_event_hooks(Some(&input)),
        BTreeSet::from([EventHook::PreCall])
    );
}

#[test]
fn normalize_event_hooks_coerces_a_list_of_hooks() {
    let input = EventHookInput::Many(vec![
        "pre_mcp_call".to_owned(),
        "during_mcp_call".to_owned(),
    ]);
    assert_eq!(
        normalize_event_hooks(Some(&input)),
        BTreeSet::from([EventHook::PreMcpCall, EventHook::DuringMcpCall])
    );
}

#[test]
fn normalize_event_hooks_discards_unparsable_entries() {
    let input = EventHookInput::Many(vec!["pre_call".to_owned(), "not_a_hook".to_owned()]);
    assert_eq!(
        normalize_event_hooks(Some(&input)),
        BTreeSet::from([EventHook::PreCall])
    );
}

#[test]
fn get_supported_event_hooks_returns_the_six_hooks() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [
            EventHook::PreCall,
            EventHook::DuringCall,
            EventHook::PostCall,
            EventHook::LoggingOnly,
            EventHook::PreMcpCall,
            EventHook::DuringMcpCall,
        ]
    );
}

#[rstest]
#[case::exclusively_mcp(&[EventHook::PreMcpCall, EventHook::DuringMcpCall], InspectionType::Chat, InspectionType::Mcp)]
#[case::exclusively_chat(&[EventHook::PreCall, EventHook::PostCall], InspectionType::Mcp, InspectionType::Chat)]
#[case::mixed_preserves_current(&[EventHook::PreCall, EventHook::PreMcpCall], InspectionType::Chat, InspectionType::Chat)]
#[case::empty_preserves_current(&[], InspectionType::Mcp, InspectionType::Mcp)]
fn infer_inspection_from_mode_flips_only_for_exclusive_surfaces(
    #[case] hooks: &[EventHook],
    #[case] current: InspectionType,
    #[case] expected: InspectionType,
) {
    let configured = BTreeSet::from_iter(hooks.iter().copied());
    assert_eq!(infer_inspection_from_mode(&configured, current), expected);
}

#[test]
fn warn_if_mode_surface_mismatch_is_silent_for_exclusive_surfaces() {
    let log_path = env::temp_dir().join(format!(
        "betterlite-cisco-mismatch-silent-{}",
        process::id()
    ));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(Arc::new(fs::File::create(&log_path).expect("log file")))
        .finish();
    let configured = BTreeSet::from([EventHook::PreCall, EventHook::PostCall]);
    let guard = set_default(subscriber);
    warn_if_mode_surface_mismatch("firewall", InspectionType::Chat, &configured);
    drop(guard);
    let written = fs::read_to_string(&log_path).expect("log contents");
    fs::remove_file(&log_path).expect("remove log");
    assert!(written.is_empty());
}

#[test]
fn warn_if_mode_surface_mismatch_names_the_unused_hooks_for_mixed_configuration() {
    let log_path =
        env::temp_dir().join(format!("betterlite-cisco-mismatch-warn-{}", process::id()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(Arc::new(fs::File::create(&log_path).expect("log file")))
        .finish();
    let configured = BTreeSet::from([EventHook::PreCall, EventHook::PreMcpCall]);
    let guard = set_default(subscriber);
    warn_if_mode_surface_mismatch("firewall", InspectionType::Chat, &configured);
    drop(guard);
    let written = fs::read_to_string(&log_path).expect("log contents");
    fs::remove_file(&log_path).expect("remove log");
    assert!(written.contains("firewall"));
    assert!(written.contains("PreMcpCall"));
}
