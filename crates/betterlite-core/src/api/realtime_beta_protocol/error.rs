use thiserror::Error;

#[derive(Debug, Error)]
pub enum BetaProtocolError {
    #[error("session must be a JSON object")]
    Session,
    #[error("session.modalities must be null or a list of strings")]
    Modalities,
    #[error("session.audio must be null or a JSON object")]
    Audio,
    #[error("event must be a JSON object")]
    Event,
    #[error("event.type must be a string")]
    EventType,
    #[error("a content block type must be a string")]
    ContentType,
}
