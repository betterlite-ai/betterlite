#[expect(
    clippy::disallowed_types,
    reason = "LiteLLM settings rows hold arbitrary JSON values"
)]
pub type JsonValue = serde_json::Value;

// LiteLLM keeps no store for ui_settings; it is only the stored row behind the general_settings UI keys
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    GeneralSettings,
    RouterSettings,
    LitellmSettings,
    EnvironmentVariables,
    UiSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DbRow {
    GeneralSettings,
    RouterSettings,
    LitellmSettings,
    EnvironmentVariables,
    UiSettings,
}

impl From<Section> for DbRow {
    fn from(section: Section) -> Self {
        match section {
            Section::GeneralSettings => Self::GeneralSettings,
            Section::RouterSettings => Self::RouterSettings,
            Section::LitellmSettings => Self::LitellmSettings,
            Section::EnvironmentVariables => Self::EnvironmentVariables,
            Section::UiSettings => Self::UiSettings,
        }
    }
}

pub const UI_SETTINGS_FIELDS: [&str; 10] = [
    "allow_public_health_readiness_details",
    "forward_client_headers_to_llm_api",
    "forward_llm_provider_auth_headers",
    "disable_agents_for_internal_users",
    "allow_agents_for_team_admins",
    "disable_vector_stores_for_internal_users",
    "allow_vector_stores_for_team_admins",
    "disable_custom_api_keys",
    "disable_key_generate_for_org_admin",
    "team_admin_editable_team_fields",
];

impl Section {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::GeneralSettings => "general_settings",
            Self::RouterSettings => "router_settings",
            Self::LitellmSettings => "litellm_settings",
            Self::EnvironmentVariables => "environment_variables",
            Self::UiSettings => "ui_settings",
        }
    }

    #[must_use]
    pub fn row_for(self, key: &str) -> DbRow {
        if self == Self::GeneralSettings && UI_SETTINGS_FIELDS.contains(&key) {
            DbRow::UiSettings
        } else {
            DbRow::from(self)
        }
    }

    // pass_through_endpoints is a list of resources the proxy merges from both sources, so the file never owns it
    #[must_use]
    pub fn is_resource_list(self, key: &str) -> bool {
        match self {
            Self::GeneralSettings => key == "pass_through_endpoints",
            Self::RouterSettings
            | Self::LitellmSettings
            | Self::EnvironmentVariables
            | Self::UiSettings => false,
        }
    }
}
