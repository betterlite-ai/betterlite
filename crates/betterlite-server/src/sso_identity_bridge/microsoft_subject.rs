use betterlite_auth::identity::MicrosoftInteractiveSubject;
use uuid::Uuid;

const GRAPH_USERINFO_ENDPOINT: &str = "https://graph.microsoft.com/v1.0/me";

#[derive(Clone, Copy, Debug, Default)]
pub struct MicrosoftEndpoints<'endpoints> {
    pub authorization: Option<&'endpoints str>,
    pub token: Option<&'endpoints str>,
    pub userinfo: Option<&'endpoints str>,
}

// a custom or sovereign cloud moves the endpoints, so its subject is not bound to the public Microsoft issuer
#[must_use]
pub fn microsoft_interactive_subject(
    tenant: Option<&str>,
    response_id: Option<&str>,
    endpoints: &MicrosoftEndpoints<'_>,
) -> Option<MicrosoftInteractiveSubject> {
    let tenant_text = Uuid::parse_str(tenant?).ok()?.to_string();
    let oid = Uuid::parse_str(response_id?).ok()?.to_string();
    let authorization =
        format!("https://login.microsoftonline.com/{tenant_text}/oauth2/v2.0/authorize");
    let token = format!("https://login.microsoftonline.com/{tenant_text}/oauth2/v2.0/token");
    let expected = [
        (endpoints.authorization, authorization.as_str()),
        (endpoints.token, token.as_str()),
        (endpoints.userinfo, GRAPH_USERINFO_ENDPOINT),
    ];
    for (override_value, expected_value) in expected {
        if override_value.is_some_and(|text| !text.is_empty() && text != expected_value) {
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
