from typing import Literal, TypeAlias, TypedDict

from litellm.proxy.auth.user_api_key_auth import (
    _get_bearer_token,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L585-L610
    _get_bearer_token_or_received_api_key,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L491-L511
    is_no_auth_dev_mode,
)
from pydantic import JsonValue


class BearerCall(TypedDict):
    operation: Literal["bearer"]
    value: str


class DevModeCall(TypedDict):
    operation: Literal["dev_mode"]
    master_key: str | None
    enable_jwt_auth: bool
    enable_oauth2_auth: bool
    enable_oauth2_proxy_auth: bool


Input: TypeAlias = BearerCall | DevModeCall


def bearer_answer(value: str) -> JsonValue:
    strict: str = _get_bearer_token(value)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L585-L610
    permissive = _get_bearer_token_or_received_api_key(value)
    return {"strict": strict, "permissive": permissive}


def run(input_value: Input) -> JsonValue:
    match input_value["operation"]:
        case "bearer":
            return bearer_answer(input_value["value"])
        case "dev_mode":
            return is_no_auth_dev_mode(input_value["master_key"], input_value)
