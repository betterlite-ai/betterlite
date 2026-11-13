use secrecy::SecretString;

use crate::cyberark::operation::SecretOperation;

// LiteLLM declares the write and rotate response as a free-form dict[str, object], so the body is kept verbatim
#[derive(Debug, Clone)]
pub struct VaultJsonResponse {
    pub body: SecretString,
}

#[derive(Debug, Clone)]
pub enum VaultOperationResult {
    Response(VaultJsonResponse),
    Status(SecretOperation),
}
