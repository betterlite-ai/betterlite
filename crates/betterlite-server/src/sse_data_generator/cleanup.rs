use super::stream::Session;

pub(super) fn finish<Key: Sync>(session: &mut Session<Key>, stream_completed: bool) {
    let Some(mut outcome) = session.outcome.take() else {
        return;
    };
    outcome.stream_completed = stream_completed;
    session.cleanup.finalize(outcome);
}

pub(super) fn mark_completed<Key: Sync>(session: &mut Session<Key>) {
    if let Some(outcome) = session.outcome.as_mut() {
        outcome.stream_completed = true;
    }
}

impl<Key: Sync> Drop for Session<Key> {
    fn drop(&mut self) {
        let Some(mut outcome) = self.outcome.take() else {
            return;
        };
        outcome.client_disconnected = !outcome.stream_completed;
        self.cleanup.finalize(outcome);
    }
}
