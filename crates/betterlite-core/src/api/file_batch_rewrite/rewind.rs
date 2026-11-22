use std::io::{self, Seek, SeekFrom};

use super::spool::OwnedSpool;

// LiteLLM's finally: the source goes back to 0 on success, failure and cancellation alike
pub(super) struct RewindOnDrop<'source, Source: Seek> {
    pub(super) source: &'source mut Source,
    pub(super) attempted: bool,
}

impl<Source> RewindOnDrop<'_, Source>
where
    Source: Seek,
{
    pub(super) fn rewind(&mut self) -> io::Result<()> {
        self.attempted = true;
        self.source.seek(SeekFrom::Start(0)).map(drop)
    }
}

impl<Source> Drop for RewindOnDrop<'_, Source>
where
    Source: Seek,
{
    fn drop(&mut self) {
        if self.attempted {
            return;
        }
        drop(self.source.seek(SeekFrom::Start(0)));
    }
}

// The spool stays open only when the body succeeded, an error closes it, and the source rewinds either way; a failed seek replaces that outcome
pub(super) fn settle_spool<Source, Value, Error>(
    guarded: &mut RewindOnDrop<'_, Source>,
    close_on_drop: &mut OwnedSpool,
    outcome: Result<Value, Error>,
) -> Result<Value, Error>
where
    Source: Seek,
    Error: From<io::Error>,
{
    let held = close_on_drop.0.take();
    if outcome.is_err()
        && let Some(spool) = held
    {
        spool.close();
    }
    match guarded.rewind() {
        Ok(()) => outcome,
        Err(source) => Err(Error::from(source)),
    }
}
