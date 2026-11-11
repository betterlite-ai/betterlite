#[test]
fn notice_buffer_sends_one_trimmed_notice() {
    let mut messages = RecordingMessages::default();
    let mut buffer = NoticeBuffer {
        messages: &mut messages,
        text: String::new(),
    };
    writeln!(buffer, "litellm: synced 2 proxy models into /x/models.json").unwrap();
    buffer.finish();
    assert_eq!(
        messages.notices(),
        ["litellm: synced 2 proxy models into /x/models.json"]
    );
    assert!(messages.warns().is_empty());
}

#[test]
fn notice_buffer_sends_nothing_when_no_message_was_written() {
    let mut messages = RecordingMessages::default();
    let mut buffer = NoticeBuffer {
        messages: &mut messages,
        text: String::new(),
    };
    buffer.finish();
    assert!(messages.notices().is_empty());
}

#[test]
fn notice_buffer_refuses_text_that_is_not_utf8() {
    let mut messages = RecordingMessages::default();
    let mut buffer = NoticeBuffer {
        messages: &mut messages,
        text: String::new(),
    };
    let error = buffer.write(&[0xff]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}
