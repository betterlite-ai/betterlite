use crate::native_speech::constants::{DEFAULT_RATE, MSTTS_XMLNS, SYNTHESIS_XMLNS};
use crate::native_speech::mapping::MappedParams;

#[must_use]
pub fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[must_use]
pub fn build_express_as_element(
    content: &str,
    style: Option<&str>,
    styledegree: Option<&str>,
    role: Option<&str>,
) -> String {
    let mut attributes = Vec::new();
    if let Some(style_value) = style.filter(|text| !text.is_empty()) {
        attributes.push(format!("style='{style_value}'"));
    }
    if let Some(styledegree_value) = styledegree.filter(|text| !text.is_empty()) {
        attributes.push(format!("styledegree='{styledegree_value}'"));
    }
    if let Some(role_value) = role.filter(|text| !text.is_empty()) {
        attributes.push(format!("role='{role_value}'"));
    }
    if attributes.is_empty() {
        return content.to_owned();
    }
    format!(
        "<mstts:express-as {}>{content}</mstts:express-as>",
        attributes.join(" ")
    )
}

#[must_use]
pub fn build_ssml(input: &str, voice: &str, mapped: &MappedParams) -> String {
    let rate = mapped.rate.as_deref().unwrap_or(DEFAULT_RATE);
    let style = mapped.style.as_deref();
    let styledegree = mapped.styledegree.as_deref();
    let role = mapped.role.as_deref();
    let escaped = escape_xml(input);
    let content = build_express_as_element(
        &format!("<prosody rate='{rate}'>{escaped}</prosody>"),
        style,
        styledegree,
        role,
    );
    let uses_mstts = style.is_some_and(|value| !value.is_empty())
        || styledegree.is_some_and(|value| !value.is_empty())
        || role.is_some_and(|value| !value.is_empty());
    let xmlns = if uses_mstts {
        format!("xmlns='{SYNTHESIS_XMLNS}' xmlns:mstts='{MSTTS_XMLNS}'")
    } else {
        format!("xmlns='{SYNTHESIS_XMLNS}'")
    };
    let voice_lang = mapped
        .lang
        .as_deref()
        .filter(|lang| !lang.is_empty())
        .map_or_else(String::new, |lang| format!(" xml:lang='{lang}'"));
    format!(
        "<speak version='1.0' {xmlns} xml:lang='en-US'>\n    <voice name='{voice}'{voice_lang}>\n        {content}\n    </voice>\n</speak>"
    )
}
