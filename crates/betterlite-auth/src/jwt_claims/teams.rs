use super::TypedValue;
use super::claims::{ClaimError, JwtClaims, list_first, text_list, text_value};
use super::trust::TrustClaims as _;

const TEAM_ID_CLAIM: &str = "_litellm_team_id";
const TEAM_IDS_CLAIM: &str = "_litellm_team_ids";

pub trait TeamClaims {
    fn is_required_team_id(&self) -> bool;
    fn team_ids(&self) -> Result<Vec<String>, ClaimError>;
    fn all_team_ids(&self) -> Result<Vec<String>, ClaimError>;
    fn team_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
    fn team_alias(&self, default: Option<&str>) -> Result<Option<String>, ClaimError>;
}

impl TeamClaims for JwtClaims<'_> {
    fn is_required_team_id(&self) -> bool {
        if self.settings.team_id_jwt_field.is_none() && self.settings.team_alias_jwt_field.is_none()
        {
            return false;
        }
        true
    }

    fn team_ids(&self) -> Result<Vec<String>, ClaimError> {
        if self.has_trusted_claim(TEAM_IDS_CLAIM) {
            let Some(value) = self.nested(TEAM_IDS_CLAIM)? else {
                return Ok(Vec::new());
            };
            return marker_team_ids(&value);
        }
        let Some(field) = self.settings.team_ids_jwt_field.as_deref() else {
            return Ok(Vec::new());
        };
        let Some(value) = self.nested(field)? else {
            return Ok(Vec::new());
        };
        if value.json.is_array() {
            return text_list(value, field);
        }
        Ok(Vec::new())
    }

    fn all_team_ids(&self) -> Result<Vec<String>, ClaimError> {
        let mut team_ids = self.team_ids()?;
        let Some((singular, claim)) = singular_team_claim(self)? else {
            return Ok(team_ids);
        };
        if singular.json.is_null() {
            return Ok(team_ids);
        }
        if singular.json.is_array() {
            append_list(&mut team_ids, &singular, &claim)?;
            return Ok(team_ids);
        }
        let Some(text) = text_value(singular, &claim)? else {
            return Ok(team_ids);
        };
        if !text.is_empty() && !team_ids.contains(&text) {
            team_ids.push(text);
        }
        Ok(team_ids)
    }

    fn team_id(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        if self.has_trusted_claim(TEAM_ID_CLAIM) {
            let Some(value) = self.nested(TEAM_ID_CLAIM)? else {
                return Ok(None);
            };
            return first_or_default(&value, TEAM_ID_CLAIM, default);
        }
        let Some(field) = self.settings.team_id_jwt_field.as_deref() else {
            return Ok(self.settings.team_id_default.clone());
        };
        let Some(value) = self.nested(field)? else {
            return Ok(configured_default(self, default));
        };
        if value.json.is_null() {
            return Ok(None);
        }
        first_or_default(&value, field, default)
    }

    fn team_alias(&self, default: Option<&str>) -> Result<Option<String>, ClaimError> {
        let Some(field) = self.settings.team_alias_jwt_field.as_deref() else {
            return Ok(None);
        };
        self.nested_text(field, default)
    }
}

// the singular claim the token carries: the issuer-normalized marker wins over the configured field
fn singular_team_claim(claims: &JwtClaims<'_>) -> Result<Option<(TypedValue, String)>, ClaimError> {
    if claims.has_trusted_claim(TEAM_ID_CLAIM) {
        let found = claims.nested(TEAM_ID_CLAIM)?;
        return Ok(found.map(|entry| (entry, TEAM_ID_CLAIM.to_owned())));
    }
    let Some(field) = claims.settings.team_id_jwt_field.as_deref() else {
        return Ok(None);
    };
    let found = claims.nested(field)?;
    Ok(found.map(|entry| (entry, field.to_owned())))
}

fn configured_default(claims: &JwtClaims<'_>, default: Option<&str>) -> Option<String> {
    if let Some(configured) = &claims.settings.team_id_default {
        return Some(configured.clone());
    }
    default.map(str::to_owned)
}

fn marker_team_ids(value: &TypedValue) -> Result<Vec<String>, ClaimError> {
    if value.json.is_string() {
        let found = text_value(value.clone(), TEAM_IDS_CLAIM)?;
        return Ok(found.map(|text| vec![text]).unwrap_or_default());
    }
    if value.json.is_array() {
        return text_list(value.clone(), TEAM_IDS_CLAIM);
    }
    Ok(Vec::new())
}

// a singular claim that is a list contributes its non-null, non-empty, not yet listed members in order
fn append_list(
    team_ids: &mut Vec<String>,
    value: &TypedValue,
    claim: &str,
) -> Result<(), ClaimError> {
    let Some(items) = value.json.as_array() else {
        return Ok(());
    };
    for item in items {
        if item.is_null() {
            continue;
        }
        let text =
            serde_json::from_value::<String>(item.clone()).map_err(|source| ClaimError::Type {
                claim: claim.to_owned(),
                source,
            })?;
        if !text.is_empty() && !team_ids.contains(&text) {
            team_ids.push(text);
        }
    }
    Ok(())
}

// a list claim takes its first element, an empty list and a null path fall back to the caller's default
fn first_or_default(
    value: &TypedValue,
    claim: &str,
    default: Option<&str>,
) -> Result<Option<String>, ClaimError> {
    if value.json.is_array() {
        if value.json.as_array().is_some_and(Vec::is_empty) {
            return Ok(default.map(str::to_owned));
        }
        return list_first(value, claim);
    }
    text_value(value.clone(), claim)
}
