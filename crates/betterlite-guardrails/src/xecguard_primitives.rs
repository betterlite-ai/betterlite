pub mod block_message;
pub mod error;
pub mod grounding_documents;
pub mod history;
pub mod logging_hook;
pub mod message;
pub mod redaction;
pub mod settings;
pub mod transport;
pub mod wire;

pub const DEFAULT_POLICIES: [&str; 3] = [
    "Default_Policy_SystemPromptEnforcement",
    "Default_Policy_HarmfulContentProtection",
    "Default_Policy_GeneralPromptAttackProtection",
];
pub const METADATA_GROUNDING_KEY: &str = "xecguard_grounding_documents";
pub const RATIONALE_TRUNCATE_CHARS: usize = 200;
