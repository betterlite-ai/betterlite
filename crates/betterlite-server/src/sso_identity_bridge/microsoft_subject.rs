use betterlite_auth::identity::MicrosoftInteractiveSubject;
use betterlite_auth::oauth_client::microsoft::microsoft_default_discovery;
use uuid::Uuid;

// a custom or sovereign cloud moves the endpoints, so its subject is not bound to the public Microsoft issuer
#[must_use]
pub fn microsoft_interactive_subject(
    tenant: Option<&str>,
    response_id: Option<&str>,
    overrides: &[(&str, Option<&str>)],
) -> Option<MicrosoftInteractiveSubject> {
    let tenant_text = Uuid::parse_str(tenant?).ok()?.to_string();
    let oid = Uuid::parse_str(response_id?).ok()?.to_string();
    let defaults = microsoft_default_discovery(&tenant_text);
    for (name, override_value) in overrides {
        let Some(value) = *override_value else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        // the expected map has no entry for another name, so any truthy value under one suppresses the subject
        let expected = match *name {
            "MICROSOFT_AUTHORIZATION_ENDPOINT" => defaults.authorization_endpoint.as_str(),
            "MICROSOFT_TOKEN_ENDPOINT" => defaults.token_endpoint.as_str(),
            "MICROSOFT_USERINFO_ENDPOINT" => defaults.userinfo_endpoint.as_str(),
            _ => return None,
        };
        if value != expected {
            return None;
        }
    }
    let issuer = format!("https://login.microsoftonline.com/{tenant_text}/v2.0");
    Some(MicrosoftInteractiveSubject {
        issuer,
        tenant_id: tenant_text,
        oid,
    })
}
