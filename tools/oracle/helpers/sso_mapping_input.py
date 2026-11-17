import asyncio
import os
from typing import Final, Literal, TypedDict

from litellm.proxy._types import ProxyException
from litellm.proxy.management_endpoints import ui_sso
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

CLIENT_ID: Final = "generic-client-id"
REDIRECT_URL: Final = "https://proxy.example.com/sso/generic/callback"


class RoleMappingsView(TypedDict):
    provider: str
    group_claim: str
    default_role: str | None
    roles: dict[str, list[str]]


class SettingsOut(TypedDict):
    client_secret: str
    scope: list[str]
    authorization_endpoint: str
    token_endpoint: str
    userinfo_endpoint: str
    include_client_id: bool


class Call(TypedDict):
    function: Literal["role_mappings", "headers", "env_vars"]


class Input(TypedDict):
    environment: dict[str, str]
    calls: list[Call]


SETUP_ROLE_MAPPINGS: Final = ui_sso._setup_role_mappings  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L1345
SETUP_ENV_VARS: Final = ui_sso._setup_generic_sso_env_vars  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L1250
PARSE_HEADERS: Final = ui_sso._parse_generic_sso_headers  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L1403

MAPPINGS_VIEW: Final = TypeAdapter(RoleMappingsView)
SETTINGS_VIEW: Final = TypeAdapter(SettingsOut)
OBJECT_VIEW: Final = TypeAdapter(dict[str, Json])


def role_mappings_answer() -> Json:
    loaded = asyncio.run(SETUP_ROLE_MAPPINGS())
    if loaded is None:
        return {"role_mappings": None}
    view = MAPPINGS_VIEW.validate_json(loaded.model_dump_json())
    return {"role_mappings": OBJECT_VIEW.validate_python(view)}


def headers_answer() -> Json:
    headers: dict[str, Json] = {}
    for name, value in PARSE_HEADERS().items():
        headers[name] = value
    return {"headers": headers}


def env_vars_answer() -> Json:
    try:
        resolved = SETUP_ENV_VARS(CLIENT_ID, REDIRECT_URL)
    except ProxyException as error:
        failure: dict[str, Json] = {
            "type": error.type,
            "param": error.param,
            "code": error.code,
            "message": error.message,
        }
        return {"error": failure}
    client_secret, scope, authorization, token, userinfo, include = resolved
    settings: SettingsOut = {
        "client_secret": client_secret,
        "scope": scope,
        "authorization_endpoint": authorization,
        "token_endpoint": token,
        "userinfo_endpoint": userinfo,
        "include_client_id": include,
    }
    return {"settings": SETTINGS_VIEW.validate_python(settings)}


def answer(call: Call) -> Json:
    if call["function"] == "role_mappings":
        return role_mappings_answer()
    if call["function"] == "headers":
        return headers_answer()
    return env_vars_answer()


def run(input_value: Input) -> Json:
    previous = dict(os.environ)
    os.environ.clear()
    os.environ.update(input_value["environment"])
    try:
        return {"calls": [answer(call) for call in input_value["calls"]]}
    finally:
        os.environ.clear()
        os.environ.update(previous)
