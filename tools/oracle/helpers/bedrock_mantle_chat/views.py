from collections.abc import Iterator
from typing import Final, Protocol, runtime_checkable

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

from .inputs import EnvironmentArgs, Object, UrlArgs

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)


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
        logging_obj: LoggingStub,
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
