use crate::bedrock_messages_beta::tests::support::{
    Call, Fixture, Session, capture_logs, headers, list,
};

const PREFIX: &str =
    "Bedrock Invoke: dropping unsupported anthropic-beta values from client headers";
const SUFFIX: &str = "Bedrock has no mapping entry for these; forwarding them would cause a 400.";
const ADVANCED: &str = "advanced-tool-use-2025-11-20";
const TOOL_SEARCH_BETA: &str = "tool-search-tool-2025-10-19";

fn resolve(call: &mut Call) -> Vec<String> {
    let fixture = Fixture::new();
    let mut session = Session::new(&fixture);
    session
        .betas(call.request())
        .expect("the beta headers resolve")
}

fn warning(dropped: &str) -> String {
    format!("{PREFIX}: {dropped}. {SUFFIX}")
}

fn call_with(header: &str) -> Call {
    let mut call = Call::default();
    call.model = "fixture-tool-search".to_owned();
    call.subclass_provider = Some("bedrock_mantle".to_owned());
    call.header_values = headers(&format!(r#"{{"anthropic-beta": "{header}"}}"#));
    call
}

pub(super) fn run() {
    let (_guard, logs) = capture_logs();

    // only the betas the provider can never map are reported, sorted like Python renders them
    let mut call = call_with("advanced-tool-use-2025-11-20,mystery-beta,");
    assert_eq!(resolve(&mut call), [TOOL_SEARCH_BETA]);
    assert_eq!(logs.warnings(), [warning("['', 'mystery-beta']")]);

    // a beta that maps to another nonempty value is forwarded, not reported
    let (_second_guard, second_logs) = capture_logs();
    let mut call = call_with(ADVANCED);
    assert_eq!(resolve(&mut call), [TOOL_SEARCH_BETA]);
    assert!(second_logs.warnings().is_empty());

    // everything dropped: the whole sorted list is reported and no beta survives
    let (_third_guard, third_logs) = capture_logs();
    let mut call = call_with("nope,alsonope");
    assert!(resolve(&mut call).is_empty());
    assert_eq!(third_logs.warnings(), [warning("['alsonope', 'nope']")]);

    // repeated values are one entry of the user set
    let (_fourth_guard, fourth_logs) = capture_logs();
    let mut call = call_with("nope,nope");
    assert!(resolve(&mut call).is_empty());
    assert_eq!(fourth_logs.warnings(), [warning("['nope']")]);

    // generated betas the provider drops are never named in the warning
    let (_fifth_guard, fifth_logs) = capture_logs();
    let mut call = call_with("nope");
    call.subclass_provider = None;
    call.model = "claude-sonnet-4-5".to_owned();
    call.messages = list(
        r#"[{"role": "user", "content": [{"type": "file", "file": {"file_id": "file-abc"}}]}]"#,
    );
    assert!(resolve(&mut call).is_empty());
    assert_eq!(fifth_logs.warnings(), [warning("['nope']")]);
}
