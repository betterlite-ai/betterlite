use std::io::{self, Seek, SeekFrom};

use tokio_util::task::TaskTracker;

use super::body::UploadBody;
use super::spool::OwnedSpool;

// LiteLLM's finally: the source goes back to 0 through the blocking port on success and failure, and is queued there on cancellation
pub(super) struct RewindOnDrop<'pass, Source>
where
    Source: Seek + Send + 'static,
{
    pub(super) body: &'pass UploadBody<Source>,
    pub(super) tasks: &'pass TaskTracker,
    pub(super) attempted: bool,
}

impl<Source> RewindOnDrop<'_, Source>
where
    Source: Seek + Send + 'static,
{
    pub(super) async fn rewind(&mut self) -> io::Result<()> {
        self.attempted = true;
        self.body.rewind(self.tasks).await
    }
}

impl<Source> Drop for RewindOnDrop<'_, Source>
where
    Source: Seek + Send + 'static,
{
    fn drop(&mut self) {
        if self.attempted {
            return;
        }
        let body = self.body.clone();
        drop(
            self.tasks
                .spawn_blocking(move || body.lock().seek(SeekFrom::Start(0)).map(drop)),
        );
    }
}

// The spool stays open only when the body succeeded, an error closes it, and the source rewinds either way; a failed seek replaces that outcome
pub(super) async fn settle_spool<Source, Value, Error>(
    guarded: &mut RewindOnDrop<'_, Source>,
    close_on_drop: &mut OwnedSpool,
    outcome: Result<Value, Error>,
) -> Result<Value, Error>
where
    Source: Seek + Send + 'static,
    Error: From<io::Error>,
{
    let held = close_on_drop.0.take();
    if outcome.is_err()
        && let Some(spool) = held
    {
        spool.close();
    }
    match guarded.rewind().await {
        Ok(()) => outcome,
        Err(source) => Err(Error::from(source)),
    }
}
