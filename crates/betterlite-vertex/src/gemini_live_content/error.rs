use thiserror::Error;

use crate::gemini_live_setup::error::LiveSetupError;

#[derive(Debug, Error)]
pub enum ContentError {
    #[error("the Gemini Live model turn is not a JSON object")]
    ModelTurnNotObject,
    #[error("the Gemini Live model turn has no parts")]
    MissingParts,
    #[error("the Gemini Live model turn parts are not a list")]
    PartsNotList,
    #[error("the Gemini Live model turn has no part to read")]
    EmptyParts,
    #[error("a Gemini Live model turn part is not a JSON object")]
    PartNotObject,
    #[error("a Gemini Live model turn part is neither text nor inline data")]
    UnexpectedPart,
    #[error("a Gemini Live model turn text or inline data value is not a string")]
    PartValueNotString,
    #[error("the Gemini Live server content is not a JSON object")]
    ContentNotObject,
    #[error("the Gemini Live event has no prior text or audio delta type")]
    MissingDeltaType,
    #[error("no OpenAI realtime event maps to the Gemini Live key `{0}`")]
    UnknownEvent(String),
    #[error("the Gemini Live frame carries no serverContent")]
    MissingServerContent,
    #[error("a text delta chunk is not a JSON object")]
    DeltaChunkNotObject,
    #[error("a text delta chunk carries no string delta")]
    MissingDeltaField,
    #[error(transparent)]
    Setup(#[from] LiveSetupError),
}
