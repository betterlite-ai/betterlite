use std::collections::BTreeMap;

use crate::jwt_claims::TypedValue;
use crate::jwt_claims::decode::unverified_claims;

// _decode_sso_token_claims: an opaque or malformed token adds no claims, since the exchange already returned it
#[must_use]
pub fn decode_sso_token_claims(token: Option<&str>) -> BTreeMap<String, TypedValue> {
    let Some(payload) = token.and_then(unverified_claims) else {
        return BTreeMap::new();
    };
    let Some(fields) = payload.json.as_object() else {
        return BTreeMap::new();
    };
    fields
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                TypedValue {
                    json: value.clone(),
                },
            )
        })
        .collect()
}

// _merge_sso_token_claims: userinfo wins over the id token and then the access token, but a null value falls through
#[must_use]
pub fn merge_sso_token_claims(
    userinfo: &BTreeMap<String, TypedValue>,
    id_token: Option<&str>,
    access_token: Option<&str>,
) -> BTreeMap<String, TypedValue> {
    let id_claims = decode_sso_token_claims(id_token);
    let access_claims = decode_sso_token_claims(access_token);
    let mut merged: BTreeMap<String, TypedValue> = BTreeMap::new();
    for source in [userinfo, &id_claims, &access_claims] {
        for (name, value) in source {
            let replace = merged
                .get(name)
                .is_none_or(|existing| existing.json.is_null());
            if replace {
                merged.insert(name.clone(), value.clone());
            }
        }
    }
    merged
}
