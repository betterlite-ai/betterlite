use super::stream::Session;

pub(super) fn finish<Key>(session: &mut Session<Key>, stream_completed: bool)
where
    Key: Sync,
{
    let Some(mut outcome) = session.outcome.take() else {
        return;
    };
    outcome.stream_completed = stream_completed;
    session.cleanup.finalize(outcome);
}

pub(super) fn mark_completed<Key>(session: &mut Session<Key>)
where
    Key: Sync,
{
    if let Some(outcome) = session.outcome.as_mut() {
        outcome.stream_completed = true;
    }
}

impl<Key> Drop for Session<Key>
where
    Key: Sync,
{
    fn drop(&mut self) {
        let Some(mut outcome) = self.outcome.take() else {
            return;
        };
        outcome.client_disconnected = !outcome.stream_completed;
        self.cleanup.finalize(outcome);
    }
}
