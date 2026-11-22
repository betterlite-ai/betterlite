use std::borrow::Cow;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use unicode_general_category::{GeneralCategory, get_general_category};

use crate::resolved_key::auth::UserAPIKeyAuth;

// _with_client_context: auth rejections run before the request setup records who called, so the failure hook gets both here
#[must_use]
pub fn with_client_context<'data>(
    request_data: &'data JsonObject,
    requester_ip: Option<&str>,
    user_agent: Option<&str>,
) -> Cow<'data, JsonObject> {
    let key = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let mut metadata = match request_data.get(key) {
        Some(Json::Object(base)) => base.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => JsonObject::new(),
    };
    let mut stamped = false;
    for (name, value) in [
        ("requester_ip_address", requester_ip),
        ("user_agent", user_agent),
    ] {
        let Some(text) = value.filter(|given| !given.is_empty()) else {
            continue;
        };
        let occupied = match metadata.get(name) {
            None | Some(Json::Null) => false,
            Some(Json::Text(existing)) => !existing.is_empty(),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => true,
        };
        if !occupied {
            metadata.insert(name.to_owned(), Json::Text(text.to_owned()));
            stamped = true;
        }
    }
    if !stamped {
        return Cow::Borrowed(request_data);
    }
    let mut copy = request_data.clone();
    copy.insert(key.to_owned(), Json::Object(metadata));
    Cow::Owned(copy)
}

// str.isprintable keeps a plain space and drops every Other and Separator category; unicode_escape spells what it drops
#[must_use]
pub fn escape_control_chars(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        let printable = character == ' '
            || !matches!(
                get_general_category(character),
                GeneralCategory::Control
                    | GeneralCategory::Format
                    | GeneralCategory::Surrogate
                    | GeneralCategory::PrivateUse
                    | GeneralCategory::Unassigned
                    | GeneralCategory::LineSeparator
                    | GeneralCategory::ParagraphSeparator
                    | GeneralCategory::SpaceSeparator
            );
        let code = u32::from(character);
        let spelled = match character {
            _ if printable => {
                escaped.push(character);
                continue;
            }
            '\t' => "\\t".to_owned(),
            '\n' => "\\n".to_owned(),
            '\r' => "\\r".to_owned(),
            '\0'..='\u{ff}' => format!("\\x{code:02x}"),
            '\u{100}'..='\u{ffff}' => format!("\\u{code:04x}"),
            _ => format!("\\U{code:08x}"),
        };
        escaped.push_str(&spelled);
    }
    escaped
}

#[must_use]
pub fn identity_log_suffix(identity: Option<&UserAPIKeyAuth>, enabled: bool) -> String {
    let Some(resolved) = identity.filter(|_| enabled) else {
        return String::new();
    };
    let token = &resolved.view.token;
    let fields = [
        ("key_alias", token.key_alias.as_deref()),
        ("user_id", token.user_id.as_deref()),
        ("user_email", resolved.extra.user_email.as_deref()),
        ("team_id", token.team_id.as_deref()),
        ("team_alias", resolved.view.team_alias.as_deref()),
    ];
    let known: Vec<String> = fields
        .into_iter()
        .filter_map(|(name, value)| {
            value
                .filter(|text| !text.is_empty())
                .map(|text| format!("{name}={}", escape_control_chars(text)))
        })
        .collect();
    if known.is_empty() {
        return String::new();
    }
    format!("\nKey Identity: {}", known.join(" "))
}
