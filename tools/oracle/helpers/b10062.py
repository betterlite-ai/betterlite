import os
from typing import Final, TypedDict

from litellm.proxy._experimental.mcp_server import oauth_utils
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

NATIVE_ENV: Final = "MCP_TRUSTED_NATIVE_REDIRECT_URIS"

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Input(TypedDict):
    native_redirect_uris: str | None
    uris: list[str]


class RefusalDetail(TypedDict):
    error: str


DETAIL: Final[TypeAdapter[RefusalDetail]] = TypeAdapter(RefusalDetail)


def refusal(error: HTTPException) -> dict[str, JsonValue]:
    detail = DETAIL.validate_python(error.detail)
    return {"status": error.status_code, "detail_error": detail["error"]}


def answer(uri: str) -> JsonValue:
    try:
        parsed = oauth_utils._parse_redirect_uri_for_validation(uri)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth_utils.py#L428
    except HTTPException as error:
        return JSON.validate_python({"refusal": refusal(error)})
    loopback = oauth_utils.is_loopback_redirect_host(parsed)
    try:
        native = oauth_utils.validate_redirect_uri_shape(parsed)
    except HTTPException as error:
        return JSON.validate_python({"loopback": loopback, "refusal": refusal(error)})
    result = "native_callback" if native else "needs_trust_policy"
    return JSON.validate_python({"loopback": loopback, "result": {result: True}})


def run(input_value: Input) -> JsonValue:
    configured = input_value["native_redirect_uris"]
    if configured is None:
        os.environ.pop(NATIVE_ENV, None)
    else:
        os.environ[NATIVE_ENV] = configured
    callbacks = oauth_utils._parse_trusted_native_redirect_uris()  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth_utils.py#L373
    answers = [answer(uri) for uri in input_value["uris"]]
    return JSON.validate_python({"native_callbacks": callbacks, "answers": answers})
