use super::support::{CountingIds, ids, json, projection, session, tag};
use crate::realtime_output::events::{AudioOutput, OutputIds, ResponseDelta, TextOutput};

fn text(content: &str) -> TextOutput {
    TextOutput {
        content: content.to_owned(),
    }
}

fn delta(event: u128, content: &str) -> String {
    format!(
        r#"{{"type":"response.text.delta","content_index":0,"event_id":"{}","item_id":"item_a","output_index":0,"response_id":"resp_a","delta":"{content}"}}"#,
        tag("event", event),
    )
}

fn assert_text_is_skipped(current: &OutputIds) {
    let source = CountingIds::new();
    let mut state = session();
    let mut absent = None;
    let event = projection(&mut state, &source).transform_text_output_event(
        &text("lost"),
        current,
        &mut absent,
    );
    assert_eq!(event, None, "{current:?}");
    assert_eq!(absent, None, "{current:?}");
    let mut kept: Option<Vec<ResponseDelta>> = Some(Vec::new());
    projection(&mut state, &source).transform_text_output_event(&text("lost"), current, &mut kept);
    assert_eq!(kept, Some(Vec::new()), "{current:?}");
    assert_eq!(source.used(), 0, "{current:?}");
}

pub(super) fn final_text() {
    assert_text_is_skipped(&OutputIds::default());
    assert_text_is_skipped(&ids(Some("resp_a"), None));
    assert_text_is_skipped(&ids(None, Some("item_a")));
    assert_text_is_skipped(&ids(Some(""), Some("item_a")));
    assert_text_is_skipped(&ids(Some("resp_a"), Some("")));
    let source = CountingIds::new();
    let mut state = session();
    let current = ids(Some("resp_a"), Some("item_a"));
    let mut chunks = None;
    let output = projection(&mut state, &source);
    let first = output.transform_text_output_event(&text("Hey"), &current, &mut chunks);
    assert_eq!(first.as_ref().map(json), Some(delta(1, "Hey")));
    let second = output.transform_text_output_event(&text(""), &current, &mut chunks);
    assert_eq!(second.as_ref().map(json), Some(delta(2, "")));
    let defaulted: TextOutput = serde_json::from_str("{}").expect("an empty textOutput");
    output.transform_text_output_event(&defaulted, &current, &mut chunks);
    let contents: Vec<String> = chunks
        .iter()
        .flatten()
        .map(|chunk| chunk.delta.clone())
        .collect();
    assert_eq!(contents, ["Hey", "", ""]);
    assert_eq!(chunks.as_ref().map(Vec::len), Some(3));
}

pub(super) fn audio_output_delta() {
    let source = CountingIds::new();
    let mut state = session();
    let current = ids(Some("resp_a"), Some("item_a"));
    let output = projection(&mut state, &source);
    let audio = AudioOutput {
        content: "!!not base64!!".to_owned(),
    };
    let event = output.transform_audio_output_event(&audio, &current);
    let expected =
        delta(1, "!!not base64!!").replace("response.text.delta", "response.audio.delta");
    assert_eq!(event.as_ref().map(json), Some(expected));
    let empty = AudioOutput::default();
    assert!(
        output
            .transform_audio_output_event(&empty, &current)
            .is_some()
    );
    for missing in [
        OutputIds::default(),
        ids(Some("resp_a"), None),
        ids(None, Some("item_a")),
        ids(Some(""), Some("item_a")),
        ids(Some("resp_a"), Some("")),
    ] {
        assert_eq!(output.transform_audio_output_event(&audio, &missing), None);
    }
    assert_eq!(source.used(), 2);
}
