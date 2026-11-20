from collections.abc import Iterator
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.chat.mantle.transformation import AmazonMantleConfig
from litellm.llms.bedrock.messages.mantle_transformation import (
    AmazonMantleMessagesConfig,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import Unpack

ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)

Object: TypeAlias = dict[str, JsonValue]


class UrlCall(TypedDict):
    model: str
    api_base: str | None
    optional_params: Object
    headers: dict[str, str]
    metadata: dict[str, str]
    project_id: str | None
    policy_fields: list[str] | None
    environment: dict[str, str]


class UrlCases(TypedDict):
    function: Literal["chat_url", "message_url"]
    calls: list[UrlCall]


class MessagesCall(TypedDict):
    function: Literal["messages_request"]
    model_cost: Object
    model: str
    messages: list[JsonValue]
    optional_params: Object
    request_headers: dict[str, str]


class ChatCall(TypedDict):
    function: Literal["chat_request", "remote_url_inlined"]
    model_cost: Object
    model: str
    messages: list[JsonValue]
    optional_params: Object
    litellm_params: Object
    headers: dict[str, str]
    fetch: dict[str, str]


class NativeSseCall(TypedDict):
    function: Literal["native_sse"]
    events: list[str]


class ResponseCall(TypedDict):
    function: Literal["messages_response"]
    body: str
    status: int


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


@runtime_checkable
class ChatView(Protocol):
    def validate_environment(
        self, **arguments: Unpack[EnvironmentArgs]
    ) -> dict[str, str]: ...

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
        *,
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
        logging_obj: Logging,
    ) -> Object: ...


class ChatProbe(AmazonMantleConfig):
    def __init__(self) -> None:
        BaseAWSLLM.__init__(self)


class MessagesProbe(AmazonMantleMessagesConfig):
    def __init__(self) -> None:
        BaseAWSLLM.__init__(self)


CHAT_PROBE: Final = ChatProbe()
CHAT: Final[ChatView] = TypeAdapter(ChatView, config=ARBITRARY).validate_python(
    CHAT_PROBE
)
MESSAGES: Final[MessagesView] = TypeAdapter(
    MessagesView, config=ARBITRARY
).validate_python(MessagesProbe())

OBJECT: Final[TypeAdapter[Object]] = TypeAdapter(Object)
