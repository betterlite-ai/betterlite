import asyncio
import time
import uuid
from typing import Final, TypeAlias, cast
from unittest import mock

from litellm._uuid import uuid as litellm_uuid  # pyright: ignore[reportPrivateUsage]
from litellm.litellm_core_utils.mock_functions import mock_embedding
from litellm.main import mock_completion  # pyright: ignore[reportUnknownVariableType]
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.common_utils.openai_error_payload import (  # pyright: ignore[reportUnknownVariableType]
    error_status_code,
    openai_error_param,
    openai_error_type,
)
from litellm.proxy.litellm_pre_call_utils import add_litellm_data_to_request  # pyright: ignore[reportUnknownVariableType]
from litellm.proxy.proxy_server import ProxyConfig  # pyright: ignore[reportPrivateUsage]
from litellm.types.utils import ModelResponse, ModelResponseStream
from litellm.utils import mock_completion_streaming_obj
from starlette.requests import Request

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

MESSAGES: Final[list[Json]] = [{"role": "user", "content": "hi"}]
REQUEST_SCOPE: Final[dict[str, object]] = {
    "type": "http", "method": "POST", "path": "/v1/chat/completions", "headers": [],
}


def run(input_value: dict[str, Json]) -> Json:
    try:
        mode = _string(input_value, "mode")
        if mode == "embedding":
            return _embedding_outcome(input_value)
        if mode not in ("chat", "stream"):
            raise ValueError(f"unknown mock_calls mode: {mode}")
        return _chat_outcome(input_value, stream=mode == "stream")
    except Exception as error:  # noqa: BLE001 - a mock failure is compared as data, not raised
        return _error_facts(error)


def _error_facts(error: BaseException) -> Json:
    status = error_status_code(error, 500)
    return {
        "kind": "error", "status": status, "type": openai_error_type(error, status),
        "param": openai_error_param(error), "code": str(status),
    }

def _embedding_outcome(input_value: dict[str, Json]) -> Json:
    response, _ = _effective_mock(input_value)
    if response is None:
        return {"kind": "no_mock"}
    vectors = cast(list[float] | None, response)
    answer = mock_embedding(model=_string(input_value, "model"), mock_response=vectors)
    return {"kind": "embedding", "response": cast(Json, answer.model_dump())}


def _chat_outcome(input_value: dict[str, Json], *, stream: bool) -> Json:
    response, tool_calls = _effective_mock(input_value)
    mock_timeout = input_value.get("mock_timeout")
    if response is None and tool_calls is None and mock_timeout is not True:
        return {"kind": "no_mock"}
    model = _string(input_value, "model")
    n = cast(int | None, input_value.get("n"))
    prompt_tokens = cast(int | None, input_value.get("prompt_tokens"))
    calls = cast(list[object] | None, tool_calls)
    answer = cast(str | dict[str, object] | None, response)
    clock = cast(int | None, input_value.get("clock"))
    if clock is None:
        raise ValueError("clock must be an integer")
    identifier = uuid.UUID(_string(input_value, "uuid"))
    kwargs: dict[str, object] = {
        "n": n, "mock_response": answer, "mock_tool_calls": calls, "mock_timeout": mock_timeout,
        "timeout": input_value.get("timeout"), "prompt_tokens": prompt_tokens,
        "mock_delay": input_value.get("mock_delay"),
    }
    with (
        mock.patch.object(time, "time", return_value=float(clock)),
        mock.patch.object(litellm_uuid, "uuid4", return_value=identifier),
    ):
        if stream and not isinstance(answer, dict):
            # mock_completion raises the sentinel errors and the mock timeout before a stream starts
            mock_completion(model, MESSAGES, stream=False, **kwargs)  # pyright: ignore[reportArgumentType]
            content = answer if isinstance(answer, str) else "This is a mock request"
            generator = mock_completion_streaming_obj(
                ModelResponseStream(), content, model=model, n=n, prompt_tokens=prompt_tokens
            )
            return {
                "kind": "stream",
                "chunks": [_project(chunk, drop_model=True) for chunk in generator],
            }
        result = mock_completion(model, MESSAGES, stream=stream, **kwargs)  # pyright: ignore[reportArgumentType]
        return {
            "kind": "response",
            "response": _project(cast(ModelResponse, result), drop_model=False),
        }


def _effective_mock(input_value: dict[str, Json]) -> tuple[Json, Json]:
    client: dict[str, Json] = {}
    for field in ("mock_response", "mock_tool_calls"):
        if input_value.get(f"client_{field}") is not None:
            client[field] = input_value[f"client_{field}"]
    data: dict[str, Json] = {"model": _string(input_value, "model"), "messages": MESSAGES, **client}
    if client:
        allowed = input_value.get("allow_client_mock_response") is True
        data = asyncio.run(_strip_client_fields(data, allowed=allowed))
    # the router builds {**litellm_params, ..., **kwargs}, so a surviving client field wins
    merged: dict[str, Json] = {"mock_response": input_value.get("operator_mock_response"), **data}
    return merged.get("mock_response"), merged.get("mock_tool_calls")


async def _strip_client_fields(data: dict[str, Json], *, allowed: bool) -> dict[str, Json]:
    metadata: dict[str, object] = {"allow_client_mock_response": True} if allowed else {}
    auth = UserAPIKeyAuth(api_key="oracle-key", metadata=metadata)
    request = Request(scope=REQUEST_SCOPE)
    return cast(
        dict[str, Json],
        await add_litellm_data_to_request(
            data=cast(dict[str, object], data),
            request=request,
            user_api_key_dict=auth,
            proxy_config=ProxyConfig(),
            general_settings={},
            version="1.105.1",
        ),
    )


def _project(value: ModelResponse | ModelResponseStream, *, drop_model: bool) -> Json:
    dump = value.model_dump(exclude_none=True, exclude_unset=True)  # pyright: ignore[reportUnknownMemberType]
    dumped = cast(dict[str, Json], dump)
    # LiteLLM's builder leaves model unset on chunks; the stream wrapper adds it when it serves them
    if drop_model:
        dumped.pop("model", None)
    _drop_betterlite_only_fields(dumped)
    return dumped

def _drop_betterlite_only_fields(value: Json) -> None:
    if isinstance(value, dict):
        value.pop("provider_specific_fields", None)
        for child in value.values():
            _drop_betterlite_only_fields(child)
    elif isinstance(value, list):
        for child in value:
            _drop_betterlite_only_fields(child)


def _string(input_value: dict[str, Json], name: str) -> str:
    value = input_value.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value
