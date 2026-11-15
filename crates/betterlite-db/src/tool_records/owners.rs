use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::identity_rows::user::LiteLLMUserTable;

use super::records::{LiteLLMToolTableRow, ToolDiscoveryUser};

#[must_use]
pub fn owners_from_rows(
    tokens: &[LiteLLMVerificationToken],
    users: &[LiteLLMUserTable],
) -> BTreeMap<String, ToolDiscoveryUser> {
    let users_by_id: BTreeMap<&str, &LiteLLMUserTable> = users
        .iter()
        .map(|user| (user.user_id.as_str(), user))
        .collect();
    let mut owners = BTreeMap::new();
    for key in tokens {
        let (Some(token), Some(user_id)) = (key.token.as_ref(), key.user_id.as_deref()) else {
            continue;
        };
        // LiteLLM keeps an owner only when both the key's token and its user id are set and non-empty
        if token.expose_secret().is_empty() || user_id.is_empty() {
            continue;
        }
        let Some(user) = users_by_id.get(user_id) else {
            continue;
        };
        owners.insert(
            token.expose_secret().to_owned(),
            ToolDiscoveryUser {
                user_id: user.user_id.clone(),
                user_email: user.user_email.clone(),
                user_alias: user.user_alias.clone(),
            },
        );
    }
    owners
}

pub fn attach_owners(
    tools: &mut [LiteLLMToolTableRow],
    owners: &BTreeMap<String, ToolDiscoveryUser>,
) {
    for tool in tools {
        tool.user = tool
            .key_hash
            .as_ref()
            .and_then(|key_hash| owners.get(key_hash))
            .cloned();
    }
}
