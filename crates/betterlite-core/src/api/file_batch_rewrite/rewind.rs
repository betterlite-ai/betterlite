use std::io::{self, Seek, SeekFrom};

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
