use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;

// the subject the master-key stage stamps; LiteLLM authenticates the master key as PROXY_ADMIN
const MASTER_KEY_SUBJECT: &str = "master_key";

pub(super) enum Caller {
    Internal { user_id: Option<String> },
    Global,
}

pub(super) fn classify(principal: &Principal) -> Caller {
    if principal.subject == MASTER_KEY_SUBJECT {
        return Caller::Global;
    }
    for role in &principal.roles {
        match role {
            Role::PlatformAdmin | Role::PlatformViewer | Role::OrgAdmin => return Caller::Global,
            Role::OrgViewer | Role::TeamAdmin | Role::TeamMember => {}
        }
    }
    Caller::Internal {
        user_id: principal.user.as_ref().map(|user| user.id.clone()),
    }
}
