import os
from typing import Final, TypedDict

from litellm.proxy.auth.login_utils import is_env_credential_login_enabled
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

SSO_VARIABLES: Final = (
    "GENERIC_AUTHORIZATION_ENDPOINT",
    "GENERIC_CLIENT_ID",
    "GENERIC_CLIENT_SECRET",
    "GENERIC_TOKEN_ENDPOINT",
    "GENERIC_USERINFO_ENDPOINT",
    "GOOGLE_CLIENT_ID",
    "GOOGLE_CLIENT_SECRET",
    "MICROSOFT_CLIENT_ID",
    "MICROSOFT_CLIENT_SECRET",
    "MICROSOFT_TENANT",
    "SAML_IDP_METADATA_URL",
    "SAML_IDP_METADATA_XML",
)

# a made-up pair makes the Google provider fully configured without a real app behind it
READY_GOOGLE_CREDENTIALS: Final[dict[str, str]] = {
    "GOOGLE_CLIENT_ID": "made-up-client-id",
    "GOOGLE_CLIENT_SECRET": "made-up-client-secret",
}


class GeneralSettings(TypedDict, total=False):
    disable_env_credential_login: bool | None
    # ast-grep-ignore: no-long-names
    disable_password_login_when_sso_enabled: bool | None


class Call(TypedDict):
    general_settings: GeneralSettings
    sso_provider_ready: bool


class Input(TypedDict):
    calls: list[Call]


def answer(call: Call) -> bool:
    saved: dict[str, str | None] = {}
    for name in SSO_VARIABLES:
        saved[name] = os.environ.pop(name, None)
    if call["sso_provider_ready"]:
        os.environ.update(READY_GOOGLE_CREDENTIALS)
    try:
        return is_env_credential_login_enabled(call["general_settings"])
    finally:
        for name in SSO_VARIABLES:
            os.environ.pop(name, None)
        for name, value in saved.items():
            if value is not None:
                os.environ[name] = value


def run(input_value: Input) -> JsonValue:
    answers: list[bool] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
