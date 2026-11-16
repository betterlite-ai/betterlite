use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReportBlocked {
    #[error(
        "GitHub could not read any selected repository. No new report was published; check repository access or try analysis again later."
    )]
    NoRepositoryReadable,
    #[error(
        "GitHub could not read {repos}, and the accessible repositories returned no pull requests. No new report was published; check repository access or try analysis again later."
    )]
    NoPullRequests { repos: String },
    #[error(
        "GitHub could not provide PR metadata. No new report was published; try analysis again later."
    )]
    NoMetadata,
    #[error(
        "The estimator could not score any pull requests. No new report was published; check the estimator connection or try analysis again later."
    )]
    NoEstimates,
}

#[derive(Debug, Error)]
pub enum CacheWriteError<Source> {
    #[error("the pull record could not be encoded for the cache")]
    Encode(#[source] serde_json::Error),
    #[error("the pull record could not be written to the cache")]
    Store(#[source] Source),
}
