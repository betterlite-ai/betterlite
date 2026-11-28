use axum::extract::Extension;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::api::claude_plugin_policy::{PluginCaller, PluginObjectPermission};
use serde::Deserialize;

use super::error::ClaudePluginReadError;
use crate::auth::auth_identity::AuthContext;

// the key record keeps its fields crate-private, so its grants are read from the serde form
#[derive(Deserialize)]
struct KeyPermissionView {
    #[serde(default)]
    object_permission: Option<PermissionSkills>,
    #[serde(default)]
    team_object_permission: Option<PermissionSkills>,
}

#[derive(Deserialize)]
struct PermissionSkills {
    #[serde(default)]
    skills: Option<Vec<String>>,
}

struct KeyPermissions {
    object: Option<PluginObjectPermission>,
    team: Option<PluginObjectPermission>,
}

pub(super) fn caller(
    context: Option<Extension<AuthContext>>,
) -> Result<Option<PluginCaller>, ClaudePluginReadError> {
    let Some(Extension(identity)) = context else {
        return Ok(None);
    };
    let granted = permissions(&identity.principal)?;
    Ok(Some(PluginCaller {
        principal: identity.principal,
        object_permission: granted.object,
        team_object_permission: granted.team,
    }))
}

fn permissions(principal: &Principal) -> Result<KeyPermissions, ClaudePluginReadError> {
    let Some(key) = principal.source_key.0.as_ref() else {
        return Ok(KeyPermissions {
            object: None,
            team: None,
        });
    };
    let value =
        serde_json::to_value(key).map_err(|source| ClaudePluginReadError::KeyView { source })?;
    let view: KeyPermissionView = serde_json::from_value(value)
        .map_err(|source| ClaudePluginReadError::KeyView { source })?;
    Ok(KeyPermissions {
        object: view
            .object_permission
            .map(|granted| PluginObjectPermission {
                skills: granted.skills,
            }),
        team: view
            .team_object_permission
            .map(|granted| PluginObjectPermission {
                skills: granted.skills,
            }),
    })
}
