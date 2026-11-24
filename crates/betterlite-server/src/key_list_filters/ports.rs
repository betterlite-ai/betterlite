use async_trait::async_trait;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::key_hash::KeyHash;

use super::error::KeyListFiltersError;

// the two rows validate_key_list_check reads before it decides: B723's user row and B722's key row
#[async_trait]
pub trait KeyListFiltersStore: Send + Sync {
    async fn user_row(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, KeyListFiltersError>;

    async fn key_by_hash(
        &self,
        key_hash: &KeyHash,
    ) -> Result<Option<LiteLLMVerificationToken>, KeyListFiltersError>;
}
