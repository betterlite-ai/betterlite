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
    openai_error_param,
    openai_error_type,
)
from litellm.proxy.litellm_pre_call_utils import add_litellm_data_to_request  # pyright: ignore[reportUnknownVariableType]
from litellm.proxy.proxy_server import ProxyConfig  # pyright: ignore[reportPrivateUsage]
from litellm.types.utils import ModelResponse, ModelResponseStream
from litellm.utils import mock_completion_streaming_obj
from starlette.requests import Request

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

DEFAULT_CONTENT: Final = "This is a mock request"
MIN_ERROR_STATUS: Final = 400
MAX_ERROR_STATUS: Final = 599
DEFAULT_ERROR_STATUS: Final = 500
REQUEST_SCOPE: Final[dict[str, object]] = {
    "type": "http", "method": "POST", "path": "/v1/chat/completions", "headers": [],
}


def run(input_value: dict[str, Json]) -> Json:
    try:
        return _outcome(input_value)
    except Exception as error:  # noqa: BLE001 - a mock failure is compared as data, not raised
        return _error_facts(error)


def _outcome(input_value: dict[str, Json]) -> Json:
    mode = _string(input_value, "mode")
    if mode == "embedding":
        return _embedding_outcome(input_value)
    if mode == "stream":
        return _chat_outcome(input_value, stream=True)
    if mode == "chat":
        return _chat_outcome(input_value, stream=False)
    raise ValueError(f"unknown mock_calls mode: {mode}")


def _error_facts(error: BaseException) -> Json:
    carried = cast(object, getattr(error, "status_code", None))
    status = (
        carried
        if isinstance(carried, int)
        and not isinstance(carried, bool)
        and MIN_ERROR_STATUS <= carried <= MAX_ERROR_STATUS
        else DEFAULT_ERROR_STATUS
    )
    # the proxy answers with this envelope, so the adapter compares the body facts
    return {
        "kind": "error",
        "status": status,
        "type": openai_error_type(error, status),
        "param": openai_error_param(error),
        "code": str(status),
    }


def _embedding_outcome(input_value: dict[str, Json]) -> Json:
    response, _ = _effective_mock(input_value)
    if response is None:
        return {"kind": "no_mock"}
    model = _string(input_value, "model")
    vectors = cast(list[float] | None, response)
    answer = mock_embedding(model=model, mock_response=vectors)
    return {"kind": "embedding", "response": cast(Json, answer.model_dump())}


def _chat_outcome(input_value: dict[str, Json], *, stream: bool) -> Json:
    model = _string(input_value, "model")
    response, tool_calls = _effective_mock(input_value)
    timeout_flag = input_value.get("mock_timeout")
    if timeout_flag is not None and not isinstance(timeout_flag, bool):
        raise ValueError("mock_timeout must be a boolean")
    mock_timeout = timeout_flag
    if response is None and tool_calls is None and mock_timeout is not True:
        return {"kind": "no_mock"}
    n = _optional_int(input_value, "n")
    prompt_tokens = _optional_int(input_value, "prompt_tokens")
    timeout_value = input_value.get("timeout")
    if timeout_value is None:
        timeout = None
    elif isinstance(timeout_value, bool) or not isinstance(timeout_value, (int, float, str)):
        raise ValueError("timeout must be a number or a string")
    elif isinstance(timeout_value, str):
        timeout = timeout_value
    else:
        timeout = float(timeout_value)
    delay_value = input_value.get("mock_delay")
    if delay_value is None:
        mock_delay = None
    elif isinstance(delay_value, bool) or not isinstance(delay_value, (int, float)):
        raise ValueError("mock_delay must be a number")
    else:
        mock_delay = float(delay_value)
    calls = cast(list[object] | None, tool_calls)
    response_value = cast(str | dict[str, object] | None, response)
    clock_value = input_value.get("clock")
    if not isinstance(clock_value, int) or isinstance(clock_value, bool):
        raise ValueError("clock must be an integer")
    identifier = _string(input_value, "uuid")
    with (
        mock.patch.object(time, "time", return_value=float(clock_value)),
        mock.patch.object(litellm_uuid, "uuid4", return_value=uuid.UUID(identifier)),
    ):
        if stream and not isinstance(response_value, dict):
            # mock_completion raises the sentinel errors and the mock timeout before a stream starts
            mock_completion(
                model,
                _messages(),
                stream=False,
                n=n,
                mock_response=response_value,
                mock_tool_calls=calls,
                mock_timeout=mock_timeout,
                timeout=timeout,
                prompt_tokens=prompt_tokens,
                mock_delay=mock_delay,
            )
            content = response_value if isinstance(response_value, str) else DEFAULT_CONTENT
            generator = mock_completion_streaming_obj(
                ModelResponseStream(),
                content,
                model=model,
                n=n,
                prompt_tokens=prompt_tokens,
            )
            return {
                "kind": "stream",
                "chunks": [_project_chunk(chunk) for chunk in generator],
            }
        result = mock_completion(
            model,
            _messages(),
            stream=stream,
            n=n,
            mock_response=response_value,
            mock_tool_calls=calls,
            mock_timeout=mock_timeout,
            timeout=timeout,
            prompt_tokens=prompt_tokens,
            mock_delay=mock_delay,
        )
        return {"kind": "response", "response": _project_response(cast(ModelResponse, result))}


def _effective_mock(input_value: dict[str, Json]) -> tuple[Json, Json]:
    deployment: dict[str, Json] = {}
    operator = input_value.get("operator_mock_response")
    if operator is not None:
        deployment["mock_response"] = operator
    data: dict[str, Json] = {"model": _string(input_value, "model"), "messages": _messages()}
    client_response = input_value.get("client_mock_response")
    client_tool_calls = input_value.get("client_mock_tool_calls")
    if client_response is not None:
        data["mock_response"] = client_response
    if client_tool_calls is not None:
        data["mock_tool_calls"] = client_tool_calls
    if "mock_response" in data or "mock_tool_calls" in data:
        allowed = input_value.get("allow_client_mock_response") is True
        data = asyncio.run(_strip_client_fields(data, allowed=allowed))
    # the router builds {**litellm_params, ..., **kwargs}, so a surviving client field wins
    merged = {**deployment, **data}
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


def _messages() -> list[Json]:
    return [{"role": "user", "content": "hi"}]


def _project_response(response: ModelResponse) -> Json:
    dumped = cast(
        Json,
        response.model_dump(  # pyright: ignore[reportUnknownMemberType]
            exclude_none=True, exclude_unset=True
        ),
    )
    _drop_betterlite_only_fields(dumped)
    return dumped


def _project_chunk(chunk: ModelResponseStream) -> Json:
    dumped = cast(
        dict[str, Json],
        chunk.model_dump(  # pyright: ignore[reportUnknownMemberType]
            exclude_none=True, exclude_unset=True
        ),
    )
    # LiteLLM's builder leaves model unset; the stream wrapper adds it when it serves the chunk
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


def _optional_int(input_value: dict[str, Json], name: str) -> int | None:
    value = input_value.get(name)
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value
