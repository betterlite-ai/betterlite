#![cfg(test)]

use crate::api::realtime_session_state::{ConnectionSetup, RealtimeStreaming};

use self::support::{
    APPEND, CLEAR_FRAME, COMMIT_FRAME, CapturedLogs, END_FRAME, Fixture, MALFORMED_FRAME,
    NON_OBJECT_FRAME, Sender, UNRELATED_FRAME, append, audio_frame, byte_sum, capturing, make,
    no_guardrails,
};
use super::{
    MAX_BUFFERED_BYTES, MAX_BUFFERED_MESSAGES, RealtimeSetupBufferError, SetupBuffer as _,
    SetupFlush as _, restore_unsent_messages,
};

mod support;

const FAILED_FLUSH_LOG: &str = "DEBUG Failed to flush buffered client message after setup: the realtime backend send failed (2 buffered message(s) retained)";

fn captured_warning(logs: &CapturedLogs, expected: &str) {
    assert!(logs.take().iter().any(|line| line.as_str() == expected));
}

#[tokio::test]
async fn deferred_audio_buffer_predicate() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    assert!(state.uses_deferred_backend_setup());
    assert_audio_frames_buffer(&mut state);
    assert_predicate_edges(&mut state);

    state.setup.backend_setup_complete.0 = false;
    state.setup.flushing_pending_messages.0 = false;
    let (first, second) = (append("first"), COMMIT_FRAME.to_owned());
    state.buffer_pending_message(&first).unwrap();
    state.buffer_pending_message(&second).unwrap();
    assert!(fixture.script().seen.is_empty());
    assert_eq!(fixture.backend_log.attempts(), 0);
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    assert!(sender.flush_pending_messages().await.unwrap());
    assert_eq!(fixture.script().seen, vec![first.clone(), second.clone()]);
    assert_eq!(fixture.backend_log.sent_text(), vec![first, second]);
    assert_immediate_setup_never_buffers();
}

fn assert_audio_frames_buffer(state: &mut RealtimeStreaming) {
    let audio = [
        append("a"),
        COMMIT_FRAME.to_owned(),
        CLEAR_FRAME.to_owned(),
        END_FRAME.to_owned(),
    ];
    for frame in &audio {
        assert!(state.should_buffer_until_setup(frame).unwrap());
    }
    assert!(!state.should_buffer_until_setup(UNRELATED_FRAME).unwrap());
    assert!(!state.should_buffer_until_setup(MALFORMED_FRAME).unwrap());
    assert!(!state.should_buffer_until_setup(r#"{"type":7}"#).unwrap());
    assert!(matches!(
        state.should_buffer_until_setup(NON_OBJECT_FRAME),
        Err(RealtimeSetupBufferError::NotAnObject)
    ));
    assert!(matches!(
        state.buffer_pending_message(NON_OBJECT_FRAME),
        Err(RealtimeSetupBufferError::NotAnObject)
    ));
}

fn assert_predicate_edges(state: &mut RealtimeStreaming) {
    state.setup.backend_setup_complete.0 = true;
    assert!(!state.should_buffer_until_setup(&append("b")).unwrap());
    state.setup.flushing_pending_messages.0 = true;
    assert!(state.should_buffer_until_setup(&append("b")).unwrap());
}

fn assert_immediate_setup_never_buffers() {
    let fixture = Fixture::new(true);
    let mut state = fixture.state();
    assert!(!state.uses_deferred_backend_setup());
    state.setup.backend_setup_complete.0 = false;
    assert!(!state.should_buffer_until_setup(&append("c")).unwrap());

    let mut bare = RealtimeStreaming::new(ConnectionSetup::default());
    assert!(!bare.uses_deferred_backend_setup());
    bare.setup.backend_setup_complete.0 = false;
    assert!(!bare.should_buffer_until_setup(&append("c")).unwrap());
}

#[tokio::test]
async fn deferred_audio_clear_order() {
    assert_clear_collapses_buffered_frames().await;
    assert_clear_keeps_non_audio_frames(UNRELATED_FRAME).await;
    assert_clear_keeps_non_audio_frames(MALFORMED_FRAME).await;
    assert_clear_on_empty_queue_sends_nothing().await;
    assert_clear_accounts_utf8_bytes();
}

async fn assert_clear_collapses_buffered_frames() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    let (first, second, third, fourth) = (append("a"), append("b"), append("c"), append("d"));
    let frames = [
        first.as_str(),
        second.as_str(),
        CLEAR_FRAME,
        third.as_str(),
        COMMIT_FRAME,
        fourth.as_str(),
        CLEAR_FRAME,
        END_FRAME,
    ];
    for frame in frames {
        state.buffer_pending_message(frame).unwrap();
    }
    let expected = vec![third.clone(), COMMIT_FRAME.to_owned(), END_FRAME.to_owned()];
    assert_eq!(state.setup.pending_messages_until_setup, expected);
    assert_eq!(state.setup.pending_messages_byte_total, byte_sum(&expected));
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    assert!(sender.flush_pending_messages().await.unwrap());
    assert_eq!(fixture.backend_log.sent_text(), expected);
    assert!(sender.state.setup.pending_messages_until_setup.is_empty());
    assert_eq!(sender.state.setup.pending_messages_byte_total, 0);
}

async fn assert_clear_keeps_non_audio_frames(frame: &str) {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    let kept = append("kept");
    state.buffer_pending_message(&kept).unwrap();
    state.buffer_pending_message(frame).unwrap();
    state.buffer_pending_message(CLEAR_FRAME).unwrap();
    let expected = vec![kept, frame.to_owned()];
    assert_eq!(state.setup.pending_messages_until_setup, expected);
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    assert!(sender.flush_pending_messages().await.unwrap());
    assert_eq!(fixture.backend_log.sent_text(), expected);
}

async fn assert_clear_on_empty_queue_sends_nothing() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    state.buffer_pending_message(CLEAR_FRAME).unwrap();
    state.buffer_pending_message(CLEAR_FRAME).unwrap();
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    assert!(sender.flush_pending_messages().await.unwrap());
    assert_eq!(fixture.backend_log.attempts(), 0);
}

fn assert_clear_accounts_utf8_bytes() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    let accented = audio_frame("\u{e9}");
    assert!(accented.len() > accented.chars().count());
    state.buffer_pending_message(&accented).unwrap();
    state.buffer_pending_message(COMMIT_FRAME).unwrap();
    state.buffer_pending_message(&accented).unwrap();
    state.buffer_pending_message(CLEAR_FRAME).unwrap();
    let expected = vec![accented.clone(), COMMIT_FRAME.to_owned()];
    assert_eq!(state.setup.pending_messages_until_setup, expected);
    assert_eq!(state.setup.pending_messages_byte_total, byte_sum(&expected));
    assert_eq!(
        state.setup.pending_messages_byte_total,
        accented.len().saturating_add(COMMIT_FRAME.len())
    );
}

#[test]
fn deferred_audio_queue_caps() {
    assert_message_cap();
    assert_byte_cap();
}

fn assert_message_cap() {
    assert_eq!(MAX_BUFFERED_MESSAGES, 200);
    let fixture = Fixture::new(false);
    let (logs, _guard) = capturing();
    let mut state = fixture.state();
    for _ in 0..MAX_BUFFERED_MESSAGES {
        state.buffer_pending_message(APPEND).unwrap();
    }
    assert_eq!(
        state.setup.pending_messages_until_setup.len(),
        MAX_BUFFERED_MESSAGES
    );
    let bytes = APPEND.len().saturating_mul(MAX_BUFFERED_MESSAGES);
    assert_eq!(state.setup.pending_messages_byte_total, bytes);
    state.buffer_pending_message(APPEND).unwrap();
    assert_eq!(
        state.setup.pending_messages_until_setup.len(),
        MAX_BUFFERED_MESSAGES
    );
    assert_eq!(state.setup.pending_messages_byte_total, bytes);
    let warning = format!(
        "WARN Pre-setup buffer full ({MAX_BUFFERED_MESSAGES} messages / {bytes} bytes); dropping frame"
    );
    captured_warning(&logs, &warning);
    assert!(fixture.client_log.sent().is_empty());
    assert_eq!(fixture.backend_log.attempts(), 0);
    state.buffer_pending_message(CLEAR_FRAME).unwrap();
    assert!(state.setup.pending_messages_until_setup.is_empty());
    assert_eq!(state.setup.pending_messages_byte_total, 0);
    state.buffer_pending_message(APPEND).unwrap();
    assert_eq!(state.setup.pending_messages_until_setup.len(), 1);
    assert_eq!(state.setup.pending_messages_byte_total, APPEND.len());
}

fn assert_byte_cap() {
    assert_eq!(MAX_BUFFERED_BYTES, 10_485_760);
    let fixture = Fixture::new(false);
    let (logs, _guard) = capturing();
    let mut state = fixture.state();
    let overhead = audio_frame("").len();
    let padding = "a".repeat(MAX_BUFFERED_BYTES.saturating_sub(overhead));
    let exact = audio_frame(&padding);
    assert_eq!(exact.len(), MAX_BUFFERED_BYTES);
    state.buffer_pending_message(&exact).unwrap();
    assert_eq!(state.setup.pending_messages_until_setup.len(), 1);
    assert_eq!(state.setup.pending_messages_byte_total, MAX_BUFFERED_BYTES);
    state.buffer_pending_message(APPEND).unwrap();
    assert_eq!(state.setup.pending_messages_until_setup.len(), 1);
    assert_eq!(state.setup.pending_messages_byte_total, MAX_BUFFERED_BYTES);
    let warning = format!(
        "WARN Pre-setup buffer full (1 messages / {MAX_BUFFERED_BYTES} bytes); dropping frame"
    );
    captured_warning(&logs, &warning);
    assert!(fixture.client_log.sent().is_empty());
    assert_eq!(fixture.backend_log.attempts(), 0);
}

#[tokio::test]
async fn deferred_audio_flush_retry() {
    let fixture = Fixture::new(false);
    let (logs, _guard) = capturing();
    let mut state = fixture.state();
    let (first, second, third, late) = (append("a"), append("b"), append("c"), append("d"));
    state.buffer_pending_message(&first).unwrap();
    state.buffer_pending_message(&second).unwrap();
    state.buffer_pending_message(&third).unwrap();
    let unsent = vec![second.clone(), third.clone()];
    fixture.backend_log.fail_on(1);
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    assert!(!sender.flush_pending_messages().await.unwrap());
    assert_eq!(sender.state.setup.pending_messages_until_setup, unsent);
    assert_eq!(
        sender.state.setup.pending_messages_byte_total,
        byte_sum(&unsent)
    );
    captured_warning(&logs, FAILED_FLUSH_LOG);
    assert_eq!(fixture.backend_log.sent_text(), vec![first.clone()]);
    assert_arrival_restores_behind_unsent(&unsent, &late);
    assert_retry_sends_the_rest(&mut sender, &fixture, &unsent, &late).await;
    assert_eq!(
        fixture.script().seen,
        vec![
            first.clone(),
            second.clone(),
            second.clone(),
            third.clone(),
            late.clone()
        ]
    );
    assert_dropped_transform_flushes_the_rest().await;
    assert_empty_flush_sends_nothing().await;
}

// A frame that lands in the queue while the failing batch is in flight ends up behind the restored unsent frames, and the byte total follows the merge
fn assert_arrival_restores_behind_unsent(unsent: &[String], late: &str) {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    state.buffer_pending_message(late).unwrap();
    restore_unsent_messages(&mut state.setup, unsent);
    let mut retained = unsent.to_vec();
    retained.push(late.to_owned());
    assert_eq!(state.setup.pending_messages_until_setup, retained);
    assert_eq!(state.setup.pending_messages_byte_total, byte_sum(&retained));
}

async fn assert_retry_sends_the_rest(
    sender: &mut Sender<'_>,
    fixture: &Fixture,
    unsent: &[String],
    late: &str,
) {
    sender.state.buffer_pending_message(late).unwrap();
    let mut retained = unsent.to_vec();
    retained.push(late.to_owned());
    assert_eq!(sender.state.setup.pending_messages_until_setup, retained);
    assert_eq!(
        sender.state.setup.pending_messages_byte_total,
        byte_sum(&retained)
    );
    let mut sent = fixture.backend_log.sent_text();
    sent.extend(retained.iter().cloned());
    assert!(sender.flush_pending_messages().await.unwrap());
    assert_eq!(fixture.backend_log.sent_text(), sent);
    assert!(sender.state.setup.pending_messages_until_setup.is_empty());
    assert_eq!(sender.state.setup.pending_messages_byte_total, 0);
}

async fn assert_dropped_transform_flushes_the_rest() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    let (dropped, kept) = (append("dropped"), append("kept"));
    state.buffer_pending_message(&dropped).unwrap();
    state.buffer_pending_message(&kept).unwrap();
    fixture.script().transforms.push_back(Vec::new());
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    assert!(sender.flush_pending_messages().await.unwrap());
    assert_eq!(fixture.backend_log.sent_text(), vec![kept]);
    assert!(sender.state.setup.pending_messages_until_setup.is_empty());
}

async fn assert_empty_flush_sends_nothing() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    assert!(sender.flush_pending_messages().await.unwrap());
    assert_eq!(fixture.backend_log.attempts(), 0);
    assert_eq!(sender.state.setup.pending_messages_byte_total, 0);
}
