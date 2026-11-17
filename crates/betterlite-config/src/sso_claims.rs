use crate::loader::EnvironmentLookup;

mod tests;

pub const USER_ID_ENV: &str = "GENERIC_USER_ID_ATTRIBUTE";
pub const DISPLAY_NAME_ENV: &str = "GENERIC_USER_DISPLAY_NAME_ATTRIBUTE";
pub const EMAIL_ENV: &str = "GENERIC_USER_EMAIL_ATTRIBUTE";
pub const FIRST_NAME_ENV: &str = "GENERIC_USER_FIRST_NAME_ATTRIBUTE";
pub const LAST_NAME_ENV: &str = "GENERIC_USER_LAST_NAME_ATTRIBUTE";
pub const PROVIDER_ENV: &str = "GENERIC_USER_PROVIDER_ATTRIBUTE";
pub const ROLE_ENV: &str = "GENERIC_USER_ROLE_ATTRIBUTE";
pub const EXTRA_ATTRIBUTES_ENV: &str = "GENERIC_USER_EXTRA_ATTRIBUTES";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimAttributes {
    pub user_id: String,
    pub display_name: String,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub provider: String,
    pub role: String,
    // the nested claim paths to copy into extra_fields; an absent or empty list adds none
    pub extra_attributes: Vec<String>,
}

#[must_use]
pub fn load_claim_attributes<L>(lookup: &L) -> ClaimAttributes
where
    L: EnvironmentLookup,
{
    ClaimAttributes {
        user_id: attribute(lookup, USER_ID_ENV, "preferred_username"),
        display_name: attribute(lookup, DISPLAY_NAME_ENV, "sub"),
        email: attribute(lookup, EMAIL_ENV, "email"),
        first_name: attribute(lookup, FIRST_NAME_ENV, "first_name"),
        last_name: attribute(lookup, LAST_NAME_ENV, "last_name"),
        provider: attribute(lookup, PROVIDER_ENV, "provider"),
        role: attribute(lookup, ROLE_ENV, "role"),
        extra_attributes: lookup
            .lookup(EXTRA_ATTRIBUTES_ENV)
            .filter(|value| !value.is_empty())
            .map(|value| value.split(',').map(str::trim).map(str::to_owned).collect())
            .unwrap_or_default(),
    }
}

// os.getenv(name, default) for a name that is absent, null or empty: the default stands
fn attribute<L>(lookup: &L, name: &str, default: &str) -> String
where
    L: EnvironmentLookup,
{
    lookup
        .lookup(name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_owned())
}
