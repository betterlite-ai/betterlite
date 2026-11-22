from collections.abc import Callable
from typing import Final, Literal, TypeAlias, TypedDict, cast

from fastapi import HTTPException
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.google_endpoints import agents_endpoints
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Scope

IS_ADMIN: Final = cast(Callable[[UserAPIKeyAuth], bool], agents_endpoints._is_proxy_admin)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/google_endpoints/agents_endpoints.py#L34-L38
ENFORCE_KEY: Final = cast(Callable[[dict[str, JsonValue], UserAPIKeyAuth], None], agents_endpoints._enforce_caller_supplied_provider_key)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/google_endpoints/agents_endpoints.py#L41-L70
MERGE_QUERY: Final = cast(Callable[[dict[str, JsonValue], Request], dict[str, JsonValue]], agents_endpoints._merge_query_params_into_data)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/google_endpoints/agents_endpoints.py#L73-L109

AUTH: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)


class AdminCase(TypedDict):
    function: Literal["is_admin"]
    user_role: str | None


class EnforceKeyCase(TypedDict):
    function: Literal["enforce_key"]
    user_role: str | None
    data: dict[str, JsonValue]


class MergeQueryCase(TypedDict):
    function: Literal["merge_query"]
    data: dict[str, JsonValue]
    query: str


Scenario: TypeAlias = AdminCase | EnforceKeyCase | MergeQueryCase


def run(input_value: Scenario) -> JsonValue:
    if input_value["function"] == "is_admin":
        user = AUTH.validate_python({"user_role": input_value["user_role"]})
        return IS_ADMIN(user)
    if input_value["function"] == "enforce_key":
        return enforce_key_answer(input_value)
    return merge_query_answer(input_value)


def enforce_key_answer(scenario: EnforceKeyCase) -> JsonValue:
    data = dict(scenario["data"])
    user = AUTH.validate_python({"user_role": scenario["user_role"]})
    try:
        ENFORCE_KEY(data, user)
    except HTTPException as error:
        detail = cast(str, error.detail)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/google_endpoints/agents_endpoints.py#L62-L70
        return {"ok": False, "status": error.status_code, "detail": detail}
    return {"ok": True}


def merge_query_answer(scenario: MergeQueryCase) -> JsonValue:
    request = build_request(scenario["query"])
    return MERGE_QUERY(dict(scenario["data"]), request)


def build_request(query: str) -> Request:
    scope: Scope = {
        "type": "http",
        "method": "GET",
        "path": "/v1beta/agents",
        "query_string": query.encode(),
    }
    return Request(scope=scope)
