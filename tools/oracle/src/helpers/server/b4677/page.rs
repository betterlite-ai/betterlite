use serde_json::json;

const ACCESS_ITEM_OPEN: &str = "class=\"access-item\"><span class=\"check\">&#10003;</span>";
const DURATION_OPEN: &str = "selDur('";

pub(super) fn page_facts(html: &str) -> serde_json::Value {
    json!({
        "title": between(html, "<title>", "</title>"),
        "heading": between(html, "<h2 class=\"step-title\">", "<"),
        "initial": between(html, "class=\"logo logo-s\">", "<"),
        "hidden": json!({
            "client_id": hidden_value(html, "client_id"),
            "redirect_uri": hidden_value(html, "redirect_uri"),
            "code_challenge": hidden_value(html, "code_challenge"),
            "code_challenge_method": hidden_value(html, "code_challenge_method"),
            "state": hidden_value(html, "state"),
            "server_id": hidden_value(html, "server_id"),
            "duration": hidden_value(html, "duration"),
        }),
        "access_items": marked_texts(html, ACCESS_ITEM_OPEN, "</div>"),
        "help_href": help_href(html),
        "durations": marked_texts(html, DURATION_OPEN, "'"),
        "steps": json!({
            "s1": html.contains("id=\"s1\""),
            "s2": html.contains("id=\"s2\""),
        }),
        "api_key_required": html.contains("name=\"api_key\"") && html.contains("required"),
        "form_post": html.contains("method=\"POST\""),
    })
}

fn marked_texts<'page>(html: &'page str, opening: &str, closing: &str) -> Vec<&'page str> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some((_, after_open)) = rest.split_once(opening) {
        let Some((text, tail)) = after_open.split_once(closing) else {
            break;
        };
        found.push(text);
        rest = tail;
    }
    found
}

fn between<'text>(text: &'text str, start: &str, end: &str) -> Option<&'text str> {
    let (_, rest) = text.split_once(start)?;
    let (found, _) = rest.split_once(end)?;
    Some(found)
}

fn hidden_value<'page>(html: &'page str, field: &str) -> Option<&'page str> {
    let marker = format!("name=\"{field}\"");
    let (_, after_name) = html.split_once(marker.as_str())?;
    between(after_name, "value=\"", "\"")
}

fn help_href(html: &str) -> Option<&str> {
    let (_, after_class) = html.split_once("class=\"help-link\"")?;
    between(after_class, "href=\"", "\"")
}
