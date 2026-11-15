use std::iter::Peekable;
use std::str::Chars;

use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::python_json;

pub const KEY_CREATED_EMAIL_TEMPLATE: &str = concat!(
    "\n",
    "                    <img src=\"{email_logo_url}\" alt=\"LiteLLM Logo\" width=\"150\" height=\"50\" />\n",
    "\n",
    "                    <p> Hi {recipient_email}, <br/>\n",
    "        \n",
    "                    I'm happy to provide you with an OpenAI Proxy API Key, loaded with ${key_budget} per month. <br /> <br />\n",
    "\n",
    "                    <b>\n",
    "                    Key: <pre>{key_token}</pre> <br>\n",
    "                    </b>\n",
    "\n",
    "                    <h2>Usage Example</h2>\n",
    "\n",
    "                    Detailed Documentation on <a href=\"https://docs.litellm.ai/docs/proxy/user_keys\">Usage with OpenAI Python SDK, Langchain, LlamaIndex, Curl</a>\n",
    "\n",
    "                    <pre>\n",
    "\n",
    "                    import openai\n",
    "                    client = openai.OpenAI(\n",
    "                        api_key=\"{key_token}\",\n",
    "                        base_url={{base_url}}\n",
    "                    )\n",
    "\n",
    "                    response = client.chat.completions.create(\n",
    "                        model=\"gpt-3.5-turbo\", # model to send to the proxy\n",
    "                        messages = [\n",
    "                            {{\n",
    "                                \"role\": \"user\",\n",
    "                                \"content\": \"this is a test request, write a short poem\"\n",
    "                            }}\n",
    "                        ]\n",
    "                    )\n",
    "\n",
    "                    </pre>\n",
    "\n",
    "\n",
    "                    If you have any questions, please send an email to {email_support_contact} <br /> <br />\n",
    "\n",
    "                    Best, <br />\n",
    "                    The LiteLLM team <br />\n",
);

pub const USER_INVITED_EMAIL_TEMPLATE: &str = concat!(
    "\n",
    "                    <img src=\"{email_logo_url}\" alt=\"LiteLLM Logo\" width=\"150\" height=\"50\" />\n",
    "\n",
    "                    <p> Hi {recipient_email}, <br/>\n",
    "\n",
    "                    You were invited to use OpenAI Proxy API for team {team_name}  <br /> <br />\n",
    "\n",
    "                    <a href=\"{base_url}\" style=\"display: inline-block; padding: 10px 20px; background-color: #87ceeb; color: #fff; text-decoration: none; border-radius: 20px;\">Accept Invitation</a> <br /> <br />\n",
    "\n",
    "                    \n",
    "                    If you have any questions, please send an email to {email_support_contact} <br /> <br />\n",
    "\n",
    "                    Best, <br />\n",
    "                    The LiteLLM team <br />\n",
);

pub struct KeyCreatedEmail<'text> {
    pub email_logo_url: &'text str,
    pub recipient_email: Option<&'text str>,
    // ast-grep-ignore: no-float-money
    pub key_budget: Option<f64>,
    pub key_token: Option<&'text str>,
    pub base_url: &'text str,
    pub email_support_contact: &'text str,
}

pub struct UserInvitedEmail<'text> {
    pub email_logo_url: &'text str,
    pub recipient_email: Option<&'text str>,
    pub team_name: &'text str,
    pub base_url: &'text str,
    pub email_support_contact: &'text str,
}

#[must_use]
pub fn render_key_created_email(email: &KeyCreatedEmail<'_>) -> String {
    let key_budget = budget_text(email.key_budget);
    let recipient_email = email.recipient_email.unwrap_or("None");
    let key_token = email.key_token.unwrap_or("None");
    fill_template(
        KEY_CREATED_EMAIL_TEMPLATE,
        &[
            ("email_logo_url", email.email_logo_url),
            ("recipient_email", recipient_email),
            ("key_budget", key_budget.as_str()),
            ("key_token", key_token),
            ("email_support_contact", email.email_support_contact),
        ],
    )
}

#[must_use]
pub fn render_user_invited_email(email: &UserInvitedEmail<'_>) -> String {
    fill_template(
        USER_INVITED_EMAIL_TEMPLATE,
        &[
            ("email_logo_url", email.email_logo_url),
            ("recipient_email", email.recipient_email.unwrap_or("None")),
            ("team_name", email.team_name),
            ("base_url", email.base_url),
            ("email_support_contact", email.email_support_contact),
        ],
    )
}

// Python's str.format prints an absent value as None and a float with repr's shortest round-trip text
fn budget_text(budget: Option<f64>) -> String {
    let Some(value) = budget else {
        return "None".to_owned();
    };
    let Some(number) = Number::from_f64(value) else {
        return if value.is_nan() {
            "nan".to_owned()
        } else if value.is_sign_negative() {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    };
    python_json::dumps(&Json::Number(number))
}

// only the doubled braces and the single-brace placeholders LiteLLM's templates use, never a field value
fn fill_template(template: &str, fields: &[(&str, &str)]) -> String {
    let mut filled = String::with_capacity(template.len());
    let mut characters = template.chars().peekable();
    while let Some(character) = characters.next() {
        match (character, characters.peek().copied()) {
            ('{', Some('{')) | ('}', Some('}')) => {
                characters.next();
                filled.push(character);
            }
            ('{', _) => fill_placeholder(&mut filled, &mut characters, fields),
            (other, _) => filled.push(other),
        }
    }
    filled
}

fn fill_placeholder(
    filled: &mut String,
    characters: &mut Peekable<Chars<'_>>,
    fields: &[(&str, &str)],
) {
    let mut name = String::new();
    let mut closed = false;
    for next in characters.by_ref() {
        if next == '}' {
            closed = true;
            break;
        }
        name.push(next);
    }
    if let Some((_, value)) = fields.iter().find(|(field, _)| *field == name.as_str()) {
        filled.push_str(value);
    } else {
        filled.push('{');
        filled.push_str(&name);
        if closed {
            filled.push('}');
        }
    }
}
