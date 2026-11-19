import asyncio
from collections.abc import Iterator
from typing import (
    Final,
    Literal,
    NotRequired,
    Protocol,
    TypedDict,
    TypeAlias,
    runtime_checkable,
)

import httpx
import litellm
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.chat.mantle.transformation import AmazonMantleConfig
from litellm.llms.bedrock.messages.mantle_transformation import (
    AmazonMantleMessagesConfig,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import Unpack

from bedrock_claude_invoke import Fetcher, patched_url_fetchers
from bedrock_invoke_primitives.stubs import sealed_environment

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

Object: TypeAlias = dict[str, JsonValue]
ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)


class UrlCall(TypedDict):
    model: str
    api_base: NotRequired[str | None]
    optional_params: NotRequired[Object]
    headers: NotRequired[dict[str, str]]
    metadata: NotRequired[dict[str, str]]
    project_id: NotRequired[str | None]
    policy_fields: NotRequired[list[str] | None]
    environment: NotRequired[dict[str, str]]


class UrlCases(TypedDict):
    function: Literal["chat_url", "message_url"]
    calls: list[UrlCall]


class MessagesCall(TypedDict):
    function: Literal["messages_request"]
    model_cost: Object
    model: str
    messages: NotRequired[list[JsonValue]]
    optional_params: NotRequired[Object]
    request_headers: NotRequired[dict[str, str]]


class ChatCall(TypedDict):
    function: Literal["chat_request", "remote_url_inlined"]
    model_cost: Object
    model: str
    messages: NotRequired[list[JsonValue]]
    optional_params: NotRequired[Object]
    litellm_params: NotRequired[Object]
    headers: NotRequired[dict[str, str]]
    fetch: NotRequired[dict[str, str]]


class NativeSseCall(TypedDict):
    function: Literal["native_sse"]
    events: list[str]


class ResponseCall(TypedDict):
    function: Literal["messages_response"]
    body: str
    status: NotRequired[int]


Input: TypeAlias = UrlCases | MessagesCall | ChatCall | NativeSseCall | ResponseCall


class EnvironmentArgs(TypedDict):
    headers: dict[str, str]
    model: str
    messages: list[JsonValue]
    optional_params: Object
    litellm_params: Object
    api_key: str | None
    api_base: str | None


class UrlArgs(TypedDict):
    api_base: str | None
    api_key: str | None
    model: str
    optional_params: Object
    litellm_params: Object
    stream: bool | None


class DeltaView(Protocol):
    content: str | None


class ChoiceView(Protocol):
    delta: DeltaView
    finish_reason: str | None


class ChunkView(Protocol):
    choices: list[ChoiceView]


class LoggingStub:
    pass


@runtime_checkable
class ChatView(Protocol):
    def validate_environment(self, **arguments: Unpack[EnvironmentArgs]) -> dict[str, str]: ...

    def get_complete_url(self, **arguments: Unpack[UrlArgs]) -> str: ...

    def transform_request(
        self,
        model: str,
        messages: list[JsonValue],
        optional_params: Object,
        litellm_params: Object,
        headers: dict[str, str],
    ) -> Object: ...

    async def async_transform_request(
        self,
        model: str,
        messages: list[JsonValue],
        optional_params: Object,
        litellm_params: Object,
        headers: dict[str, str],
    ) -> Object: ...

    def get_model_response_iterator(
        self,
        streaming_response: Iterator[str],
        sync_stream: bool,
        json_mode: bool | None,
    ) -> Iterator[ChunkView]: ...


@runtime_checkable
class MessagesView(Protocol):
    def transform_anthropic_messages_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        messages: list[JsonValue],
        params: Object,
        extra: GenericLiteLLMParams,
        headers: dict[str, str],
    ) -> Object: ...

    def validate_anthropic_messages_environment(  # ast-grep-ignore: no-long-names
        self, **arguments: Unpack[EnvironmentArgs]
    ) -> tuple[dict[str, str], str | None]: ...

    def get_complete_url(self, **arguments: Unpack[UrlArgs]) -> str: ...

    def transform_anthropic_messages_response(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        raw_response: httpx.Response,
        logging_obj: LoggingStub,
    ) -> Object: ...


class ChatProbe(AmazonMantleConfig):
    def __init__(self) -> None:
        BaseAWSLLM.__init__(self)


class MessagesProbe(AmazonMantleMessagesConfig):
    def __init__(self) -> None:
        BaseAWSLLM.__init__(self)


CHAT: Final[ChatView] = TypeAdapter(ChatView, config=ARBITRARY).validate_python(
    ChatProbe()
)
MESSAGES: Final[MessagesView] = TypeAdapter(
    MessagesView, config=ARBITRARY
).validate_python(MessagesProbe())


OBJECT: Final[TypeAdapter[Object]] = TypeAdapter(Object)


def litellm_params_of(call: UrlCall) -> Object:
    params: Object = {}
    metadata = call.get("metadata")
    if metadata is not None:
        params["metadata"] = OBJECT.validate_python(metadata)
    project_id = call.get("project_id")
    if project_id is not None:
        params["aws_bedrock_project_id"] = project_id
    return params


def custom_stream_wrapper(config: object) -> bool:
    return getattr(config, "has_custom_stream_wrapper", None) is True


def complete_url(call: UrlCall, params: Object) -> str:
    environment = call.get("environment")
    with sealed_environment({} if environment is None else environment):
        return CHAT.get_complete_url(
            api_base=call.get("api_base"),
            api_key=None,
            model=call["model"],
            optional_params=params,
            litellm_params={},
            stream=None,
        )


def chat_url(call: UrlCall) -> JsonValue:
    params = dict(call.get("optional_params") or {})
    headers = call.get("headers")
    original = litellm.bedrock_request_metadata_fields
    litellm.bedrock_request_metadata_fields = list(call.get("policy_fields") or [])
    try:
        merged = CHAT.validate_environment(
            headers={} if headers is None else headers,
            model=call["model"],
            messages=[],
            optional_params=params,
            litellm_params=litellm_params_of(call),
            api_key=None,
            api_base=call.get("api_base"),
        )
    finally:
        litellm.bedrock_request_metadata_fields = original
    outcome: Object = {
        "url": complete_url(call, params),
        "headers": OBJECT.validate_python(merged),
    }
    return outcome


def message_url(call: UrlCall) -> JsonValue:
    params = dict(call.get("optional_params") or {})
    headers = call.get("headers")
    original = litellm.bedrock_request_metadata_fields
    litellm.bedrock_request_metadata_fields = list(call.get("policy_fields") or [])
    try:
        merged, api_base = MESSAGES.validate_anthropic_messages_environment(
            headers={} if headers is None else headers,
            model=call["model"],
            messages=[],
            optional_params=params,
            litellm_params=litellm_params_of(call),
            api_key=None,
            api_base=call.get("api_base"),
        )
    finally:
        litellm.bedrock_request_metadata_fields = original
    outcome: Object = {
        "url": complete_url(call, params),
        "headers": OBJECT.validate_python(merged),
        "api_base": api_base,
    }
    return outcome


def urls(call: UrlCases) -> JsonValue:
    answer = chat_url if call["function"] == "chat_url" else message_url
    outcome: Object = {"calls": [answer(entry) for entry in call["calls"]]}
    return outcome


def messages_request(call: MessagesCall) -> JsonValue:
    litellm.model_cost = dict(call["model_cost"])
    litellm.drop_params = False
    headers = dict(call.get("request_headers") or {})
    body = MESSAGES.transform_anthropic_messages_request(
        call["model"],
        list(call.get("messages") or []),
        dict(call.get("optional_params") or {}),
        GenericLiteLLMParams(),
        headers,
    )
    outcome: Object = {"body": body, "headers": OBJECT.validate_python(headers)}
    return outcome


def chat(call: ChatCall, *, inline: bool) -> JsonValue:
    litellm.register_model(dict(call["model_cost"]))
    fetcher = Fetcher(call.get("fetch") or {})
    headers = dict(call.get("headers") or {})
    optional = dict(call.get("optional_params") or {})
    extra = dict(call.get("litellm_params") or {})
    messages = list(call.get("messages") or [])
    with patched_url_fetchers(fetcher):
        if inline:
            body = asyncio.run(
                CHAT.async_transform_request(
                    call["model"], messages, optional, extra, headers
                )
            )
        else:
            body = CHAT.transform_request(
                call["model"], messages, optional, extra, headers
            )
    outcome: Object = {"body": body, "fetches": fetcher.calls}
    return outcome


def native_sse(call: NativeSseCall) -> JsonValue:
    # the case carries each native event's JSON; the iterator reads the SSE data: framing
    events = [f"data: {event}\n\n" for event in call["events"]]
    iterator = CHAT.get_model_response_iterator(
        iter(events), sync_stream=False, json_mode=None
    )
    chunks: list[JsonValue] = []
    for chunk in iterator:
        choice = chunk.choices[0]
        # the iterator spells "no finish reason" as an empty string; the contract says null
        view: Object = {
            "content": choice.delta.content or "",
            "finish_reason": choice.finish_reason or None,
        }
        chunks.append(view)
    outcome: Object = {
        "has_custom_stream_wrapper": custom_stream_wrapper(CHAT),
        "chunks": chunks,
    }
    return outcome


def messages_response(call: ResponseCall) -> JsonValue:
    response = MESSAGES.transform_anthropic_messages_response(
        "claude",
        httpx.Response(
            call.get("status", 200), content=call["body"].encode("utf-8")
        ),
        LoggingStub(),
    )
    outcome: Object = {"response": response}
    return outcome


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "chat_url" | "message_url":
            return urls(input_value)
        case "messages_request":
            return messages_request(input_value)
        case "chat_request":
            return chat(input_value, inline=False)
        case "remote_url_inlined":
            return chat(input_value, inline=True)
        case "native_sse":
            return native_sse(input_value)
        case "messages_response":
            return messages_response(input_value)
