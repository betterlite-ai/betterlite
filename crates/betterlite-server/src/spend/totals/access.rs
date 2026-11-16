use betterlite_auth::auth_identity::records::Principal;

// the subject the master-key stage stamps; LiteLLM authenticates the master key as PROXY_ADMIN
const MASTER_KEY_SUBJECT: &str = "master_key";

pub(super) enum Caller {
    Internal { user_id: Option<String> },
    Global,
}

pub(super) fn classify(principal: &Principal) -> Caller {
    if principal.subject == MASTER_KEY_SUBJECT || !principal.roles.is_empty() {
        return Caller::Global;
    }
    Caller::Internal {
        user_id: principal.user.as_ref().map(|user| user.id.clone()),
    }
}
