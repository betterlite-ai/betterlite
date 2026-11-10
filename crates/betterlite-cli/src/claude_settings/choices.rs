use secrecy::SecretString;

pub const STARTING_MODEL_ROLE: &str =
    "the /model picker's default row, the model Claude Code starts and resumes on";

#[derive(Debug, Clone)]
pub struct StaticToken {
    pub token: SecretString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelChoice {
    KeepModel,
    UnpinModel,
    StartOn(String),
}

impl ModelChoice {
    #[must_use]
    pub fn starting_model(&self) -> Option<&str> {
        match self {
            Self::KeepModel | Self::UnpinModel => None,
            Self::StartOn(model) => Some(model),
        }
    }
}

mod tests;
