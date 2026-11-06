import asyncio
import time
import uuid
from typing import Final, TypeAlias, cast
from unittest import mock

from litellm._uuid import uuid as litellm_uuid  # pyright: ignore[reportPrivateUsage]
from litellm.litellm_core_utils.mock_functions import mock_embedding
from litellm.main import mock_completion  # pyright: ignore[reportUnknownVariableType]
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.litellm_pre_call_utils import add_litellm_data_to_request  # pyright: ignore[reportUnknownVariableType]
from litellm.proxy.proxy_server import ProxyConfig  # pyright: ignore[reportPrivateUsage]
from litellm.types.utils import ModelResponse, ModelResponseStream
from litellm.utils import mock_completion_streaming_obj
from starlette.requests import Request

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

DEFAULT_CONTENT: Final = "This is a mock request"
REQUEST_SCOPE: Final = {
    "type": "http",
    "method": "POST",
    "path": "/v1/chat/completions",
    "headers": [(b"content-type", b"application/json")],
    "query_string": b"",
    "server": ("127.0.0.1", 4000),
    "client": ("127.0.0.1", 5000),
    "scheme": "http",
    "root_path": "",
}


def run(input_value: dict[str, Json]) -> Json:
    mode = _string(input_value, "mode")
    if mode == "embedding":
        return _embedding_outcome(input_value)
    if mode == "stream":
        return _chat_outcome(input_value, stream=True)
    if mode == "chat":
        return _chat_outcome(input_value, stream=False)
    raise ValueError(f"unknown mock_calls mode: {mode}")


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
    mock_timeout = _optional_bool(input_value, "mock_timeout")
    if response is None and tool_calls is None and mock_timeout is not True:
        return {"kind": "no_mock"}
    n = _optional_int(input_value, "n")
    prompt_tokens = _optional_int(input_value, "prompt_tokens")
    timeout = _optional_timeout(input_value)
    mock_delay = _optional_float(input_value, "mock_delay")
    calls = cast(list[object] | None, tool_calls)
    response_value = cast(str | dict[str, object] | None, response)
    clock = _integer(input_value, "clock")
    identifier = _string(input_value, "uuid")
    with (
        mock.patch.object(time, "time", return_value=float(clock)),
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
    operator = input_value.get("operator_mock_response")
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
    response = operator if operator is not None else data.get("mock_response")
    return response, data.get("mock_tool_calls")


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


def _integer(input_value: dict[str, Json], name: str) -> int:
    value = input_value.get(name)
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def _optional_int(input_value: dict[str, Json], name: str) -> int | None:
    value = input_value.get(name)
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def _optional_float(input_value: dict[str, Json], name: str) -> float | None:
    value = input_value.get(name)
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def _optional_bool(input_value: dict[str, Json], name: str) -> bool | None:
    value = input_value.get(name)
    if value is None:
        return None
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def _optional_timeout(input_value: dict[str, Json]) -> float | str | None:
    value = input_value.get("timeout")
    if value is None:
        return None
    if isinstance(value, bool):
        raise ValueError("timeout must be a number or a string")
    if isinstance(value, (int, float)):
        return float(value)
    if isinstance(value, str):
        return value
    raise ValueError("timeout must be a number or a string")
