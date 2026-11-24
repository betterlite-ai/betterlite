use std::error::Error;

// The group metadata read, then the counter read, in the order the remaining-usage method performs them.
#[derive(Debug, thiserror::Error)]
pub enum GroupUsageError<MetadataError, CounterError>
where
    MetadataError: Error + 'static,
    CounterError: Error + 'static,
{
    #[error(transparent)]
    Metadata(MetadataError),
    #[error(transparent)]
    Counter(CounterError),
}
