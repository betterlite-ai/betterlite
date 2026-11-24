import asyncio
import json
from collections.abc import AsyncIterator, Awaitable, Coroutine, Mapping
from typing import Final, Protocol, TypeAlias, TypedDict

from pydantic import JsonValue, TypeAdapter

from betterlite._endpoints import CallValue
from betterlite._native import (
    NativeA2aClient,
    NativeA2aError,
    create_client,
    get_agent_card,
    send_message_direct,
    send_message_streaming_direct,
)
from betterlite._settings.env import env_number
from betterlite.a2a_protocol.errors import UnsupportedA2AArgument, raise_native_error
from betterlite.a2a_protocol.types import (
    LiteLLMSendMessageResponse,
    SendMessageRequest,
    SendStreamingMessageRequest,
    SendStreamingMessageResponse,
)
from betterlite.types.utils.base import ResponseObject

DEFAULT_A2A_AGENT_TIMEOUT: Final[float] = env_number("DEFAULT_A2A_AGENT_TIMEOUT", 6000.0, float)

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
JSON_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])

A2AClientType: TypeAlias = NativeA2aClient


class SDKClient(Protocol): ...


class SDKRequest(Protocol): ...


class A2AStream(Protocol):
    def anext_chunk(self) -> Awaitable[str | None]: ...

    def aclose(self) -> Awaitable[None]: ...


class CallField(TypedDict):
    api_base: str | None
    litellm_params: dict[str, JsonValue] | None
    agent_id: str | None
    agent_extra_headers: dict[str, str] | None
    metadata: JsonValue


RequestValue: TypeAlias = (
    SendMessageRequest | SendStreamingMessageRequest | Mapping[str, JsonValue] | SDKRequest | None
)
ClientValue: TypeAlias = A2AClientType | SDKClient | None


# The @client wrapper's logging kwargs have no Python object here; the JSON ones cross into Rust
def json_kwarg(kwargs: Mapping[str, CallValue], name: str) -> JsonValue:
    value = kwargs.get(name)
    match value:
        case list() | dict():
            return JSON.validate_python(value)
        case bool() | int() | float() | str() | None:
            return value
        case object():
            return None


def text_kwarg(kwargs: Mapping[str, CallValue], name: str) -> str | None:
    match kwargs.get(name):
        case str() as text:
            return text
        case object():
            return None


def request_value(request: RequestValue) -> dict[str, JsonValue] | None:
    match request:
        case None:
            return None
        case SendMessageRequest() | SendStreamingMessageRequest():
            return request.model_dump(mode="json", exclude_none=True)
        case dict():
            return request
        case object():
            raise UnsupportedA2AArgument("request")


def native_client(client: ClientValue) -> A2AClientType | None:
    match client:
        case None:
            return None
        case NativeA2aClient():
            return client
        case object():
            raise UnsupportedA2AArgument("a2a_client")


def client_settings(
    base_url: str,
    timeout: float,
    extra_headers: dict[str, str] | None,
    *,
    streaming: bool,
    relative_card_path: str | None,
) -> str:
    settings: dict[str, JsonValue] = {
        "base_url": base_url,
        "timeout": timeout,
        "extra_headers": extra_headers,
        "streaming": streaming,
        "relative_card_path": relative_card_path,
    }
    return json.dumps(settings)


def call_text(
    request: dict[str, JsonValue] | None,
    fields: CallField,
    extras: Mapping[str, CallValue],
) -> str:
    call: dict[str, JsonValue] = {
        "request": request,
        "api_base": fields["api_base"],
        "litellm_params": fields["litellm_params"],
        "agent_id": fields["agent_id"],
        "agent_extra_headers": fields["agent_extra_headers"],
        "litellm_call_id": json_kwarg(extras, "litellm_call_id"),
        "litellm_trace_id": text_kwarg(extras, "litellm_trace_id"),
        "metadata": fields["metadata"],
    }
    return json.dumps(call)


async def asend_message(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/a2a_protocol/main.py#L404
    a2a_client: ClientValue = None,
    request: RequestValue = None,
    api_base: str | None = None,
    litellm_params: dict[str, JsonValue] | None = None,
    agent_id: str | None = None,
    agent_extra_headers: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> LiteLLMSendMessageResponse:
    client = native_client(a2a_client)
    fields = CallField(
        api_base=api_base,
        litellm_params=litellm_params,
        agent_id=agent_id,
        agent_extra_headers=agent_extra_headers,
        metadata=json_kwarg(kwargs, "metadata"),
    )
    text = call_text(request_value(request), fields, kwargs)
    answer: str
    try:
        if client is None:
            answer = await send_message_direct(text)
        else:
            answer = await client.send_message(text)
    except NativeA2aError as error:
        raise_native_error(error)
    data = JSON_OBJECT.validate_json(answer)
    # the native bridge adds the private bag LiteLLM's own response dump never carries
    _ = data.pop("_hidden_params", None)
    return LiteLLMSendMessageResponse.from_dict(data)


async def asend_message_streaming(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/a2a_protocol/main.py#L627
    a2a_client: ClientValue = None,
    request: RequestValue = None,
    api_base: str | None = None,
    litellm_params: dict[str, JsonValue] | None = None,
    agent_id: str | None = None,
    metadata: dict[str, JsonValue] | None = None,
    proxy_server_request: dict[str, JsonValue] | None = None,
    agent_extra_headers: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> AsyncIterator[SendStreamingMessageResponse]:
    _ = proxy_server_request
    client = native_client(a2a_client)
    fields = CallField(
        api_base=api_base,
        litellm_params=litellm_params,
        agent_id=agent_id,
        agent_extra_headers=agent_extra_headers,
        metadata=metadata,
    )
    text = call_text(request_value(request), fields, kwargs)
    stream: A2AStream
    try:
        if client is None:
            stream = await send_message_streaming_direct(text)
        else:
            stream = await client.send_message_streaming(text)
    except NativeA2aError as error:
        raise_native_error(error)
    try:
        while True:
            chunk: str | None
            try:
                chunk = await stream.anext_chunk()
            except NativeA2aError as error:
                raise_native_error(error)
            if chunk is None:
                return
            yield SendStreamingMessageResponse.model_validate_json(chunk)
    finally:
        await stream.aclose()


async def create_a2a_client(  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/a2a_protocol/main.py#L767
    base_url: str,
    timeout: float = DEFAULT_A2A_AGENT_TIMEOUT,
    extra_headers: dict[str, str] | None = None,
    streaming: bool = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/a2a_protocol/main.py#L771
    relative_card_path: str | None = None,
) -> A2AClientType:
    settings = client_settings(
        base_url,
        timeout,
        extra_headers,
        streaming=streaming,
        relative_card_path=relative_card_path,
    )
    client: A2AClientType
    try:
        client = await create_client(settings)
    except NativeA2aError as error:
        raise_native_error(error)
    return client


async def aget_agent_card(  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/a2a_protocol/main.py#L848
    base_url: str,
    timeout: float = DEFAULT_A2A_AGENT_TIMEOUT,
    extra_headers: dict[str, str] | None = None,
    relative_card_path: str | None = None,
) -> ResponseObject:
    settings = client_settings(
        base_url,
        timeout,
        extra_headers,
        streaming=False,
        relative_card_path=relative_card_path,
    )
    text: str
    try:
        text = await get_agent_card(settings)
    except NativeA2aError as error:
        raise_native_error(error)
    return ResponseObject.model_validate_json(text)


def send_message(
    a2a_client: ClientValue,
    request: RequestValue,
    **kwargs: CallValue,
) -> LiteLLMSendMessageResponse | Coroutine[None, None, LiteLLMSendMessageResponse]:
    try:
        _ = asyncio.get_running_loop()
    except RuntimeError:
        return asyncio.run(asend_message(a2a_client=a2a_client, request=request, **kwargs))
    return asend_message(a2a_client=a2a_client, request=request, **kwargs)
