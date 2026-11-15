from typing import Literal, TypedDict

from litellm.integrations.email_templates.templates import (
    KEY_CREATED_EMAIL_TEMPLATE,
    USER_INVITED_EMAIL_TEMPLATE,
)


class Input(TypedDict):
    template: Literal["key_created", "user_invited"]
    email_logo_url: str
    recipient_email: str | None
    key_budget: float | None
    key_token: str | None
    team_name: str | None
    base_url: str
    email_support_contact: str


def run(input_value: Input) -> dict[str, str]:
    if input_value["template"] == "key_created":
        return {
            "html": KEY_CREATED_EMAIL_TEMPLATE.format(
                email_logo_url=input_value["email_logo_url"],
                recipient_email=input_value["recipient_email"],
                key_budget=input_value["key_budget"],
                key_token=input_value["key_token"],
                base_url=input_value["base_url"],
                email_support_contact=input_value["email_support_contact"],
            )
        }
    return {
        "html": USER_INVITED_EMAIL_TEMPLATE.format(
            email_logo_url=input_value["email_logo_url"],
            recipient_email=input_value["recipient_email"],
            team_name=input_value["team_name"],
            base_url=input_value["base_url"],
            email_support_contact=input_value["email_support_contact"],
        )
    }
