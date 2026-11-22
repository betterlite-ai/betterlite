// the mock-side plumbing the file read branches need: the azure client a handler would build, the logging state a pre-call records into, the anthropic config a generic branch hands B2462's handler, and the transport a supplied handler lends
pub(super) mod azure;
pub(super) mod logging;
pub(super) mod metadata;
pub(super) mod transport;
