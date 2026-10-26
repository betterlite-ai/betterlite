use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "LaxBool")]
pub struct ScimBool(pub bool);
// Pydantic's lax mode reads these spellings as booleans, and directories send them
#[derive(Deserialize)]
#[serde(untagged)]
enum LaxBool {
    Flag(bool),
    Number(i64),
    Text(String),
}
#[derive(Debug, Error)]
#[error("the value is not a valid boolean")]
pub struct BoolError;
impl TryFrom<LaxBool> for ScimBool {
    type Error = BoolError;
    fn try_from(input: LaxBool) -> Result<Self, BoolError> {
        match input {
            LaxBool::Flag(flag) => Ok(Self(flag)),
            LaxBool::Number(0) => Ok(Self(false)),
            LaxBool::Number(1) => Ok(Self(true)),
            LaxBool::Text(text) => match text.to_ascii_lowercase().as_str() {
                "0" | "f" | "n" | "no" | "off" | "false" => Ok(Self(false)),
                "1" | "t" | "y" | "on" | "yes" | "true" => Ok(Self(true)),
                _ => Err(BoolError),
            },
            LaxBool::Number(_) => Err(BoolError),
        }
    }
}
