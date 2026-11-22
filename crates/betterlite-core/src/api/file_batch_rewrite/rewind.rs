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

// The scan and the rewrite both rewind once their outcome is known, and a failed seek replaces that outcome
pub(super) fn finish<Source, Value, Error>(
    guarded: &mut RewindOnDrop<'_, Source>,
    result: Result<Value, Error>,
) -> Result<Value, Error>
where
    Source: Seek,
    Error: From<io::Error>,
{
    match (result, guarded.rewind()) {
        (_, Err(source)) => Err(Error::from(source)),
        (value, Ok(())) => value,
    }
}

// The spool stays open only when the body succeeded, an error closes it, and the source rewinds either way
pub(super) fn settle_spool<Source, Value, Error>(
    guarded: &mut RewindOnDrop<'_, Source>,
    close_on_drop: &mut OwnedSpool,
    outcome: Result<Value, Error>,
) -> Result<Value, Error>
where
    Source: Seek,
    Error: From<io::Error>,
{
    match outcome {
        Ok(value) => {
            drop(close_on_drop.0.take());
            finish(guarded, Ok(value))
        }
        Err(error) => {
            if let Some(spool) = close_on_drop.0.take() {
                spool.close();
            }
            finish(guarded, Err(error))
        }
    }
}
