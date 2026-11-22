import json
from typing import ClassVar

from pydantic import BaseModel, ConfigDict, JsonValue

from betterlite._completion.arguments import JSON
from betterlite._completion.errors import raise_native_error
from betterlite._native import NativeError, NativeSession


class TokenCountResponse(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(protected_namespaces=())

    total_tokens: int
    request_model: str
    model_used: str
    tokenizer_type: str
    original_response: dict[str, JsonValue] | None = None
    error: bool = False
    error_message: str | None = None
    status_code: int | None = None


def _probe_request(model: str, api_key: str | None) -> str:
    request: dict[str, JsonValue] = {"model": model}
    if api_key is not None:
        request["api_key"] = api_key
    return json.dumps(request)


def valid_model(model: str) -> None:
    try:
        NativeSession("{}").valid_model(_probe_request(model, None))
    except NativeError as error:
        raise_native_error(error)


def check_valid_key(model: str, api_key: str) -> bool:
    try:
        return NativeSession("{}").check_valid_key(_probe_request(model, api_key))
    except NativeError:
        return False


async def acount_tokens(
    model: str,
    messages: list[dict[str, JsonValue]] | None = None,
    tools: list[dict[str, JsonValue]] | None = None,
    system: str | None = None,
    api_key: str | None = None,
    api_base: str | None = None,
) -> TokenCountResponse:
    request: dict[str, JsonValue] = {"model": model}
    if messages is not None:
        request["messages"] = JSON.validate_python(messages)
    if tools is not None:
        request["tools"] = JSON.validate_python(tools)
    if system is not None:
        request["system"] = system
    if api_key is not None:
        request["api_key"] = api_key
    if api_base is not None:
        request["api_base"] = api_base
    session = NativeSession("{}")
    try:
        answer = session.count_tokens(json.dumps(request))
    except NativeError as error:
        raise_native_error(error)
    return TokenCountResponse.model_validate_json(answer)
