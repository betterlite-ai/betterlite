use std::collections::BTreeMap;

use betterlite_config::sections::team_admin_fields::EDITABLE_FIELDS_SETTING;
use betterlite_config::ui_settings::{
    CLIENT_HEADERS_SETTING, DISABLE_AGENTS_SETTING, DISABLE_CUSTOM_KEYS_SETTING,
    DISABLE_MODEL_ADD_SETTING, DISABLE_ORG_KEY_GENERATE, DISABLE_TEAM_DELETE_SETTING,
    DISABLE_VECTOR_STORES_SETTING, ENABLE_CHAT_UI_SETTING, HEALTH_DETAILS_SETTING, PAGES_SETTING,
    PROVIDER_HEADERS_SETTING, PUBLIC_AI_HUB_AUTH_SETTING, TEAM_ADMIN_AGENTS_SETTING,
    TEAM_ADMIN_VECTOR_STORES, USER_SEARCH_SCOPE_SETTING,
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct UiFieldItems {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(rename = "enum", skip_serializing_if = "Option::is_none")]
    pub values: Option<&'static [&'static str]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct UiFieldSchema {
    pub description: &'static str,
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<UiFieldItems>,
}

pub type UiSettingsSchema = BTreeMap<String, UiFieldSchema>;

const TEAM_VALUES: &[&str] = &[
    "max_budget",
    "member_key_budgets",
    "projects",
    "rpm_limit",
    "tpm_limit",
];

const PAGES_ITEMS: UiFieldItems = UiFieldItems {
    kind: "string",
    values: None,
};

const TEAM_ITEMS: UiFieldItems = UiFieldItems {
    kind: "string",
    values: Some(TEAM_VALUES),
};

struct UiFieldSpec {
    name: &'static str,
    description: &'static str,
    kind: &'static str,
    items: Option<UiFieldItems>,
}

const fn boolean_field(name: &'static str, description: &'static str) -> UiFieldSpec {
    UiFieldSpec {
        name,
        description,
        kind: "boolean",
        items: None,
    }
}

const UI_SETTINGS_FIELD_SPECS: [UiFieldSpec; 16] = [
    boolean_field(
        DISABLE_MODEL_ADD_SETTING,
        "If true, internal users cannot add models from the UI",
    ),
    boolean_field(
        DISABLE_TEAM_DELETE_SETTING,
        "Prevents Team Admins from deleting users from the teams they manage. Useful for SCIM provisioning where team membership is defined externally.",
    ),
    UiFieldSpec {
        name: PAGES_SETTING,
        description: "List of page keys that internal users (non-admins) can see in the UI sidebar. If not set, all pages are visible based on role permissions.",
        kind: "array",
        items: Some(PAGES_ITEMS),
    },
    boolean_field(
        PUBLIC_AI_HUB_AUTH_SETTING,
        "If true, requires authentication for accessing the public AI Hub.",
    ),
    boolean_field(
        HEALTH_DETAILS_SETTING,
        "If true, returns the legacy detailed payload from the unauthenticated /health/readiness endpoint.",
    ),
    boolean_field(
        CLIENT_HEADERS_SETTING,
        "Forwards client headers (Authorization, anthropic-beta, and x-* custom headers) to the upstream LLM. Enable for Claude Code with a Max subscription (forwards the OAuth token) or to pass custom/tracing headers through to the provider. Independent of the BYOK toggle \u{2014} enable only the one(s) you need.",
    ),
    boolean_field(
        PROVIDER_HEADERS_SETTING,
        "Forwards provider auth headers (x-api-key, x-goog-api-key, api-key, ocp-apim-subscription-key) to the upstream LLM, overriding any deployment-configured key for that request. Enable for Claude Code BYOK (clients bring their own API key). Independent of the client-headers toggle \u{2014} enable only the one(s) you need.",
    ),
    boolean_field(
        DISABLE_AGENTS_SETTING,
        "If true, internal users cannot access agent management endpoints or the Agents page in the UI.",
    ),
    boolean_field(
        TEAM_ADMIN_AGENTS_SETTING,
        "If true, team admins are exempt from the agents disable restriction (only takes effect when disable_agents_for_internal_users is true).",
    ),
    boolean_field(
        DISABLE_VECTOR_STORES_SETTING,
        "If true, internal users cannot access vector store management endpoints or the Vector Stores page in the UI.",
    ),
    boolean_field(
        TEAM_ADMIN_VECTOR_STORES,
        "If true, team admins are exempt from the vector stores disable restriction (only takes effect when disable_vector_stores_for_internal_users is true).",
    ),
    boolean_field(
        USER_SEARCH_SCOPE_SETTING,
        "If enabled, the user search endpoint (/user/filter/ui) restricts results by organization. When off, any authenticated user can search all users.",
    ),
    boolean_field(
        DISABLE_CUSTOM_KEYS_SETTING,
        "If true, users cannot specify custom key values. All keys must be auto-generated.",
    ),
    boolean_field(
        DISABLE_ORG_KEY_GENERATE,
        "If true, org admins cannot generate API keys via /key/generate.",
    ),
    boolean_field(
        ENABLE_CHAT_UI_SETTING,
        "If true, shows the Chat page in the UI sidebar, letting users chat with an LLM and connect their own MCP server credentials via OAuth.",
    ),
    UiFieldSpec {
        name: EDITABLE_FIELDS_SETTING,
        description: "Team settings fields a team admin may change on the teams they administer. Include 'projects' to let team admins create and update projects for those teams. Include 'member_key_budgets' to let team admins update budget fields on keys owned by other members of those teams. Empty means team admins cannot edit team settings or manage projects at all. Proxy admins and org admins are not affected.",
        kind: "array",
        items: Some(TEAM_ITEMS),
    },
];

// The schema entries _schema_property_entry reports for the OSS UISettings class.
#[must_use]
pub fn ui_settings_schema() -> UiSettingsSchema {
    UI_SETTINGS_FIELD_SPECS
        .iter()
        .map(|spec| {
            (
                spec.name.to_owned(),
                UiFieldSchema {
                    description: spec.description,
                    kind: spec.kind,
                    items: spec.items,
                },
            )
        })
        .collect()
}
