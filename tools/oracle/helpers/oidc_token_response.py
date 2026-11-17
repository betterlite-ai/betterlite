from typing import TypedDict, cast

import httpx
from litellm.proxy.management_endpoints.ui_sso import SSOAuthenticationHandler
from pydantic import JsonValue


class Input(TypedDict):
    body: str


def run(input_value: Input) -> JsonValue:
    response = httpx.Response(status_code=200, content=input_value["body"].encode())
    validated = SSOAuthenticationHandler._validate_token_response(response)  # type: ignore[misc]  # noqa: E501, SLF001  # pyright: ignore[reportAny, reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L3960-L4018
    return cast(JsonValue, validated)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L4018
