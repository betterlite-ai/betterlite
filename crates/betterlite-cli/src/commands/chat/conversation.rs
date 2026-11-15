use std::fs;
use std::io;

use super::Message;

pub trait ConversationStore {
    fn save(&mut self, filename: &str, messages: &[Message]) -> Result<(), ConversationError>;
    fn load(&mut self, filename: &str) -> Result<Vec<Message>, ConversationError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error("the file does not exist")]
    NotFound,
    #[error("{filename} could not be read")]
    Read {
        filename: String,
        #[source]
        source: io::Error,
    },
    #[error("{filename} could not be written")]
    Write {
        filename: String,
        #[source]
        source: io::Error,
    },
    #[error("{filename} does not hold a conversation")]
    Format {
        filename: String,
        #[source]
        source: serde_json::Error,
    },
}

pub struct FileConversationStore;

impl ConversationStore for FileConversationStore {
    #[cfg_attr(test, mutants::skip)]
    fn save(&mut self, filename: &str, messages: &[Message]) -> Result<(), ConversationError> {
        let text =
            serde_json::to_string_pretty(messages).map_err(|source| ConversationError::Format {
                filename: filename.to_owned(),
                source,
            })?;
        fs::write(filename, text).map_err(|source| ConversationError::Write {
            filename: filename.to_owned(),
            source,
        })
    }

    #[cfg_attr(test, mutants::skip)]
    fn load(&mut self, filename: &str) -> Result<Vec<Message>, ConversationError> {
        let bytes = match fs::read(filename) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(ConversationError::NotFound);
            }
            Err(source) => {
                return Err(ConversationError::Read {
                    filename: filename.to_owned(),
                    source,
                });
            }
        };
        serde_json::from_slice(&bytes).map_err(|source| ConversationError::Format {
            filename: filename.to_owned(),
            source,
        })
    }
}
