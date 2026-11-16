use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

// The UserAPIKeyAuth fields LiteLLM turns into user_api_key_* metadata, read from the principal; a field the principal lacks stays out.
#[must_use]
pub fn user_key_dict(principal: &Principal) -> Object {
    let user = principal.user.as_ref();
    let team = principal.teams.first();
    let entries = [
        ("user_id", user.map(|identity| identity.id.as_str())),
        (
            "user_email",
            user.and_then(|identity| identity.email.as_deref()),
        ),
        ("team_id", team.map(|member| member.id.as_str())),
        ("team_alias", team.and_then(|member| member.name.as_deref())),
        (
            "org_id",
            principal
                .organization
                .as_ref()
                .map(|organization| organization.id.as_str()),
        ),
        (
            "end_user_id",
            principal
                .end_user
                .as_ref()
                .map(|end_user| end_user.id.as_str()),
        ),
        ("hash", principal.credential_ref.token_id.as_deref()),
    ];
    entries
        .into_iter()
        .filter_map(|(key, value)| Some((key.to_owned(), GuardrailValue::Text(value?.to_owned()))))
        .collect()
}
