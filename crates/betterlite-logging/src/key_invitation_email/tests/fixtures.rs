#![cfg(test)]

use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::budget_email::event::{EntityType, WebhookEvent, WebhookEventType};
use crate::key_invitation_email::templates::{KeyCreatedEmail, UserInvitedEmail};

// Python's str.format output for LiteLLM's templates, kept as literals because the indented blank lines fail the trailing-whitespace check

pub(super) const KEY_CREATED_HTML: &str = concat!(
    "\n",
    "                    <img src=\"https:&#x2f;&#x2f;cdn.example&#x2f;logo.png\" alt=\"LiteLLM Logo\" width=\"150\" height=\"50\" />\n",
    "\n",
    "                    <p> Hi alice@example.com, <br/>\n",
    "        \n",
    "                    I'm happy to provide you with an OpenAI Proxy API Key, loaded with $10.0 per month. <br /> <br />\n",
    "\n",
    "                    <b>\n",
    "                    Key: <pre>sk-test-token</pre> <br>\n",
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
    "                        api_key=\"sk-test-token\",\n",
    "                        base_url={base_url}\n",
    "                    )\n",
    "\n",
    "                    response = client.chat.completions.create(\n",
    "                        model=\"gpt-3.5-turbo\", # model to send to the proxy\n",
    "                        messages = [\n",
    "                            {\n",
    "                                \"role\": \"user\",\n",
    "                                \"content\": \"this is a test request, write a short poem\"\n",
    "                            }\n",
    "                        ]\n",
    "                    )\n",
    "\n",
    "                    </pre>\n",
    "\n",
    "\n",
    "                    If you have any questions, please send an email to help@example.com <br /> <br />\n",
    "\n",
    "                    Best, <br />\n",
    "                    The LiteLLM team <br />\n",
);

pub(super) const KEY_CREATED_MISSING_HTML: &str = concat!(
    "\n",
    "                    <img src=\"https:&#x2f;&#x2f;cdn.example&#x2f;logo.png\" alt=\"LiteLLM Logo\" width=\"150\" height=\"50\" />\n",
    "\n",
    "                    <p> Hi None, <br/>\n",
    "        \n",
    "                    I'm happy to provide you with an OpenAI Proxy API Key, loaded with $None per month. <br /> <br />\n",
    "\n",
    "                    <b>\n",
    "                    Key: <pre>None</pre> <br>\n",
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
    "                        api_key=\"None\",\n",
    "                        base_url={base_url}\n",
    "                    )\n",
    "\n",
    "                    response = client.chat.completions.create(\n",
    "                        model=\"gpt-3.5-turbo\", # model to send to the proxy\n",
    "                        messages = [\n",
    "                            {\n",
    "                                \"role\": \"user\",\n",
    "                                \"content\": \"this is a test request, write a short poem\"\n",
    "                            }\n",
    "                        ]\n",
    "                    )\n",
    "\n",
    "                    </pre>\n",
    "\n",
    "\n",
    "                    If you have any questions, please send an email to help@example.com <br /> <br />\n",
    "\n",
    "                    Best, <br />\n",
    "                    The LiteLLM team <br />\n",
);

pub(super) const USER_INVITED_HTML: &str = concat!(
    "\n",
    "                    <img src=\"https:&#x2f;&#x2f;cdn.example&#x2f;logo.png\" alt=\"LiteLLM Logo\" width=\"150\" height=\"50\" />\n",
    "\n",
    "                    <p> Hi bob@example.com, <br/>\n",
    "\n",
    "                    You were invited to use OpenAI Proxy API for team Alpha  <br /> <br />\n",
    "\n",
    "                    <a href=\"https:&#x2f;&#x2f;proxy.example&#x2f;ui&#x2f;onboarding?invitation_id=inv-1\" style=\"display: inline-block; padding: 10px 20px; background-color: #87ceeb; color: #fff; text-decoration: none; border-radius: 20px;\">Accept Invitation</a> <br /> <br />\n",
    "\n",
    "                    \n",
    "                    If you have any questions, please send an email to help@example.com <br /> <br />\n",
    "\n",
    "                    Best, <br />\n",
    "                    The LiteLLM team <br />\n",
);

pub(super) fn key_created_event() -> WebhookEvent {
    WebhookEvent {
        event: WebhookEventType::KeyCreated,
        event_message: "key_created".to_owned(),
        event_group: EntityType::Key,
        spend: Decimal::ZERO,
        max_budget: Some(Decimal::from(10_i64)),
        soft_budget: None,
        token: Some(SecretString::from("sk-test-token")),
        customer_id: None,
        user_id: Some("user-1".to_owned()),
        team_id: None,
        team_alias: None,
        organization_id: None,
        user_email: Some("alice@example.com".to_owned()),
        key_alias: None,
        projected_exceeded_date: None,
        projected_spend: None,
        alert_emails: None,
        max_budget_alert_emails: None,
    }
}

pub(super) fn invited_event() -> WebhookEvent {
    let mut event = key_created_event();
    event.event = WebhookEventType::InternalUserCreated;
    event.event_message = "internal_user_created".to_owned();
    event.event_group = EntityType::User;
    event.max_budget = None;
    event.token = None;
    event.team_id = Some("team-1".to_owned());
    event.user_email = Some("bob@example.com".to_owned());
    event
}

pub(super) fn budget_email(budget: Option<f64>) -> KeyCreatedEmail<'static> {
    KeyCreatedEmail {
        email_logo_url: "https://cdn.example/logo.png",
        recipient_email: Some("alice@example.com"),
        key_budget: budget,
        key_token: Some("sk-test-token"),
        base_url: "https://proxy.example",
        email_support_contact: "help@example.com",
    }
}

pub(super) fn invited_email(base_url: &str) -> UserInvitedEmail<'_> {
    UserInvitedEmail {
        email_logo_url: "https://cdn.example/logo.png",
        recipient_email: Some("bob@example.com"),
        team_name: "Alpha",
        base_url,
        email_support_contact: "help@example.com",
    }
}
