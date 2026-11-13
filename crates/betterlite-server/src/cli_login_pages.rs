mod consent;
mod escape;
mod success;
mod tests;
mod verification;

const CONFIRM_INSTRUCTIONS: &str = "Confirm the verification code below to finish this login.";
const ENTER_INSTRUCTIONS: &str =
    "Enter the verification code shown in your terminal to finish this login.";

#[derive(Clone, Copy)]
pub struct ConsentPage<'page> {
    pub client_origin: &'page str,
    pub user_id: &'page str,
    pub teams: &'page [(String, String)],
    pub flow_handle: &'page str,
    pub complete_url: &'page str,
    pub cli_expiration_hours: i64,
}

#[must_use]
pub fn render_verification_page(
    verify_url: &str,
    browser_complete_token: &str,
    prefill_user_code: Option<&str>,
) -> String {
    let prefill = prefill_user_code.filter(|code| !code.is_empty());
    let (instructions, value_attribute) = prefill.map_or_else(
        || (ENTER_INSTRUCTIONS, String::new()),
        |code| {
            (
                CONFIRM_INSTRUCTIONS,
                format!(" value=\"{}\"", escape::text(code)),
            )
        },
    );
    let url = escape::text(verify_url);
    let token = escape::text(browser_complete_token);
    fill(
        verification::PAGE,
        &[
            ("VERIFY_URL", &url),
            ("BROWSER_TOKEN", &token),
            ("INSTRUCTIONS", instructions),
            ("USER_CODE_VALUE", &value_attribute),
        ],
    )
}

pub const SUCCESS_PAGE: &str = success::PAGE;

#[must_use]
pub fn render_consent_page(page: ConsentPage<'_>) -> String {
    let origin = escape::text(page.client_origin);
    let user = escape::text(page.user_id);
    let flow = escape::text(page.flow_handle);
    let complete = escape::text(page.complete_url);
    let hours = page.cli_expiration_hours.to_string();
    let team_field = consent::team_field(page.teams);
    fill(
        consent::PAGE,
        &[
            ("COMPLETE_URL", &complete),
            ("FLOW_HANDLE", &flow),
            ("ORIGIN", &origin),
            ("USER", &user),
            ("HOURS", &hours),
            ("TEAM_FIELD", &team_field),
        ],
    )
}

// a value that carries the @@ marker pair is copied once, never read as a template token
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut page = String::with_capacity(template.len());
    let mut inside_marker = false;
    for piece in template.split("@@") {
        if inside_marker {
            page.push_str(value_for(values, piece));
        } else {
            page.push_str(piece);
        }
        inside_marker = !inside_marker;
    }
    page
}

fn value_for<'value>(values: &[(&str, &'value str)], token: &'value str) -> &'value str {
    values
        .iter()
        .find(|(name, _value)| *name == token)
        .map_or(token, |(_name, value)| *value)
}
