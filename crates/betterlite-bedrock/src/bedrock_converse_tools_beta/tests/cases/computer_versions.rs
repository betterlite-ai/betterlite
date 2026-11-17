use super::super::support::{Session, list, params};
use super::{FOREIGN_MODEL, prepare};
use crate::bedrock_converse_tools_beta::convert::computer_use_beta;

const LATEST: &str = "computer-use-2025-11-24";
const MID: &str = "computer-use-2025-01-24";
const OLDER: &str = "computer-use-2024-10-22";

const LATEST_MARKERS: [&str; 16] = [
    "opus-4.7",
    "opus_4.7",
    "opus-4-7",
    "opus_4_7",
    "opus-4.6",
    "opus_4.6",
    "opus-4-6",
    "opus_4_6",
    "sonnet-4.6",
    "sonnet_4.6",
    "sonnet-4-6",
    "sonnet_4_6",
    "opus-4.5",
    "opus_4.5",
    "opus-4-5",
    "opus_4_5",
];

const MID_MARKERS: [&str; 20] = [
    "sonnet-4.5",
    "sonnet_4.5",
    "sonnet-4-5",
    "sonnet_4_5",
    "haiku-4.5",
    "haiku_4.5",
    "haiku-4-5",
    "haiku_4_5",
    "opus-4.1",
    "opus_4.1",
    "opus-4-1",
    "opus_4_1",
    "sonnet-4",
    "sonnet_4",
    "opus-4",
    "opus_4",
    "sonnet-3.7",
    "sonnet_3.7",
    "sonnet-3-7",
    "sonnet_3_7",
];

pub(super) fn version_markers() {
    let families: [(&str, &[&str]); 2] = [(LATEST, &LATEST_MARKERS), (MID, &MID_MARKERS)];
    for (expected, markers) in families {
        for marker in markers {
            assert_eq!(computer_use_beta(marker), expected, "marker {marker}");
        }
    }
}

pub(super) fn older_versions() {
    let older = [
        "anthropic.claude-3-5-sonnet-v1:0",
        "amazon.nova-pro-v1:0",
        "sonnet-3-5",
        "",
    ];
    assert_eq!(older.map(computer_use_beta), [OLDER; 4]);

    assert_eq!(computer_use_beta("ANTHROPIC.CLAUDE-OPUS-4-6-V1:0"), LATEST);
    assert_eq!(computer_use_beta("anthropic.claude-sonnet-4-5-v1:0"), MID);
    assert_eq!(computer_use_beta("anthropic.claude-opus-4-1-v1:0"), MID);
}

pub(super) fn request(session: &mut Session<'_>) {
    let processed = prepare(
        session,
        "anthropic.claude-3-5-sonnet-20241022-v2:0",
        r#"[{"type": "computer_20241022", "name": "computer", "display_width_px": 1024}, {"type": "function", "function": {"name": "lookup", "parameters": {"type": "object", "properties": {}}}}]"#,
        Some(r#"{"anthropic-beta": ["computer-use-2024-10-22", "beta-x"]}"#),
        r#"{"tools": ["old"]}"#,
    );
    assert_eq!(
        processed.tools,
        list(
            r#"[{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}, "name": "lookup", "description": "lookup"}}]"#
        )
    );
    assert_eq!(
        processed.params,
        params(
            r#"{"tools": [{"type": "computer_20241022", "name": "computer", "display_width_px": 1024}], "anthropic_beta": ["computer-use-2024-10-22", "beta-x", "computer-use-2024-10-22"]}"#
        )
    );

    let latest = prepare(
        session,
        "anthropic.claude-opus-4-6-v1:0",
        r#"[{"type": "computer_20241022", "name": "computer", "display_width_px": 1024}]"#,
        None,
        "{}",
    );
    assert!(latest.tools.is_empty());
    assert_eq!(latest.betas, list(r#"["computer-use-2025-11-24"]"#));
    assert_eq!(
        latest.params,
        params(
            r#"{"tools": [{"type": "computer_20241022", "name": "computer", "display_width_px": 1024}], "anthropic_beta": ["computer-use-2025-11-24"]}"#
        )
    );
}

pub(super) fn foreign_base(session: &mut Session<'_>) {
    let foreign = prepare(
        session,
        FOREIGN_MODEL,
        r#"[{"type": "computer_20241022", "name": "computer", "display_width_px": 1024}]"#,
        None,
        "{}",
    );
    assert_eq!(foreign.betas, list(r#"["computer-use-2024-10-22"]"#));
    assert_eq!(
        foreign.params,
        params(
            r#"{"tools": [{"type": "computer_20241022", "name": "computer", "display_width_px": 1024}]}"#
        )
    );
}
