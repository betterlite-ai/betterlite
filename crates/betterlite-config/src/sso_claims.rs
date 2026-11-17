use crate::loader::EnvironmentLookup;

mod tests;

pub const GENERIC_USER_ID_ATTRIBUTE_ENV: &str = "GENERIC_USER_ID_ATTRIBUTE";
pub const GENERIC_USER_DISPLAY_NAME_ATTRIBUTE_ENV: &str = "GENERIC_USER_DISPLAY_NAME_ATTRIBUTE";
pub const GENERIC_USER_EMAIL_ATTRIBUTE_ENV: &str = "GENERIC_USER_EMAIL_ATTRIBUTE";
pub const GENERIC_USER_FIRST_NAME_ATTRIBUTE_ENV: &str = "GENERIC_USER_FIRST_NAME_ATTRIBUTE";
pub const GENERIC_USER_LAST_NAME_ATTRIBUTE_ENV: &str = "GENERIC_USER_LAST_NAME_ATTRIBUTE";
pub const GENERIC_USER_PROVIDER_ATTRIBUTE_ENV: &str = "GENERIC_USER_PROVIDER_ATTRIBUTE";
pub const GENERIC_USER_ROLE_ATTRIBUTE_ENV: &str = "GENERIC_USER_ROLE_ATTRIBUTE";
pub const GENERIC_USER_EXTRA_ATTRIBUTES_ENV: &str = "GENERIC_USER_EXTRA_ATTRIBUTES";

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

pub fn load_claim_attributes<L>(lookup: &L) -> ClaimAttributes
where
    L: EnvironmentLookup,
{
    ClaimAttributes {
        user_id: attribute(lookup, GENERIC_USER_ID_ATTRIBUTE_ENV, "preferred_username"),
        display_name: attribute(lookup, GENERIC_USER_DISPLAY_NAME_ATTRIBUTE_ENV, "sub"),
        email: attribute(lookup, GENERIC_USER_EMAIL_ATTRIBUTE_ENV, "email"),
        first_name: attribute(lookup, GENERIC_USER_FIRST_NAME_ATTRIBUTE_ENV, "first_name"),
        last_name: attribute(lookup, GENERIC_USER_LAST_NAME_ATTRIBUTE_ENV, "last_name"),
        provider: attribute(lookup, GENERIC_USER_PROVIDER_ATTRIBUTE_ENV, "provider"),
        role: attribute(lookup, GENERIC_USER_ROLE_ATTRIBUTE_ENV, "role"),
        extra_attributes: lookup
            .lookup(GENERIC_USER_EXTRA_ATTRIBUTES_ENV)
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
