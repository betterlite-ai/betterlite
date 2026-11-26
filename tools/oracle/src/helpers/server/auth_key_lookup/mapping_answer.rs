use std::slice;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_key_lookup::jwt::{
    JwtError,
    JwtMappingSource,
    // ast-grep-ignore: no-long-names
    get_jwt_key_mapping_cache_keys_for_tokens as batch_cache_keys,
    get_jwt_key_mapping_object,
};
use betterlite_auth::auth_read::jwt_rows::{JwtMappingFilter, JwtMappingRow};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};

use super::{MappingInput, MappingLookup, MappingRow};

pub(super) async fn mapping(input: &MappingInput) -> HelperResult {
    let found = match &input.lookup {
        MappingLookup::Object { found } => found.as_ref().map(mapping_row),
        MappingLookup::Token { .. } | MappingLookup::Tokens { .. } => None,
    };
    let source = Arc::new(ScriptedMappings {
        rows: input.rows.iter().map(mapping_row).collect(),
        found,
        token_calls: Mutex::new(Vec::new()),
        filters: Mutex::new(Vec::new()),
    });
    match &input.lookup {
        MappingLookup::Token { hashed_token } => {
            let keys = batch_cache_keys(slice::from_ref(hashed_token), source.as_ref())
                .await
                .or_value_error()?;
            Ok(json!({"keys": keys, "where": source.single_where()}))
        }
        MappingLookup::Tokens { hashed_tokens } => {
            let keys = batch_cache_keys(hashed_tokens, source.as_ref())
                .await
                .or_value_error()?;
            Ok(json!({"keys": keys, "where": source.batch_where()}))
        }
        MappingLookup::Object { .. } => {
            let token = get_jwt_key_mapping_object(
                &input.claim_name,
                &input.claim_value,
                input.issuer.as_deref(),
                source.as_ref(),
            )
            .await
            .or_value_error()?;
            Ok(json!({"token": token, "where": source.object_where()}))
        }
    }
}

struct ScriptedMappings {
    rows: Vec<JwtMappingRow>,
    found: Option<JwtMappingRow>,
    token_calls: Mutex<Vec<Vec<String>>>,
    filters: Mutex<Vec<JwtMappingFilter>>,
}

impl ScriptedMappings {
    fn single_where(&self) -> Value {
        let Some(tokens) = self.tokens() else {
            return Value::Null;
        };
        tokens
            .first()
            .map_or(Value::Null, |token| json!({"token": token}))
    }

    fn batch_where(&self) -> Value {
        let Some(tokens) = self.tokens() else {
            return Value::Null;
        };
        json!({"token": {"in": tokens}})
    }

    fn object_where(&self) -> Value {
        let matched = self
            .filters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned();
        let Some(filter) = matched else {
            return Value::Null;
        };
        json!({
            "jwt_claim_name": filter.claim_name,
            "jwt_claim_value": filter.claim_value,
            "jwt_issuer": filter.issuer,
            "is_active": filter.active,
        })
    }

    fn tokens(&self) -> Option<Vec<String>> {
        self.token_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
    }
}

#[async_trait]
impl JwtMappingSource for ScriptedMappings {
    async fn find_active_mapping(
        &self,
        filter: &JwtMappingFilter,
    ) -> Result<Option<JwtMappingRow>, JwtError> {
        self.filters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(filter.clone());
        Ok(self.found.clone())
    }

    async fn mappings_for_tokens(
        &self,
        hashed_tokens: &[String],
    ) -> Result<Vec<JwtMappingRow>, JwtError> {
        self.token_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(hashed_tokens.to_vec());
        Ok(self.rows.clone())
    }
}

fn mapping_row(fields: &MappingRow) -> JwtMappingRow {
    JwtMappingRow {
        token: fields.token.clone(),
        jwt_issuer: fields.jwt_issuer.clone(),
        jwt_claim_name: fields.jwt_claim_name.clone(),
        jwt_claim_value: fields.jwt_claim_value.clone(),
    }
}
