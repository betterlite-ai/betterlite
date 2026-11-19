from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.messages.invoke_transformations.anthropic_claude3_transformation import (
    AmazonAnthropicClaudeMessagesConfig as MessagesConfig,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import Unpack

from bedrock_invoke_primitives.models import Keys

Object: TypeAlias = dict[str, JsonValue]
WARNING_PREFIXES: Final = ("Dropping ", "Bedrock Invoke: ")
REQUEST_BODY: Final[Object] = {"max_tokens": 8}
ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)


class InvokeUrl(TypedDict):
    function: Literal["invoke_url"]
    model: str
    credentials: Keys
    now: int
    api_base: NotRequired[str | None]
    stream: NotRequired[bool | None]
    fake_stream: NotRequired[bool | None]
    optional_params: NotRequired[Object]
    headers: NotRequired[dict[str, str]]
    metadata: NotRequired[Object]
    spend_logs_metadata: NotRequired[Object]
    policy_fields: NotRequired[list[str] | None]
    api_key: NotRequired[str | None]
    configured_bearer_token: NotRequired[str | None]
    request_data: NotRequired[Object]
    environment: NotRequired[dict[str, str]]


class TransformCall(TypedDict):
    model: str
    messages: NotRequired[list[JsonValue]]
    optional_params: NotRequired[Object]
    headers: NotRequired[dict[str, str]]


class Transform(TypedDict):
    function: Literal["transform"]
    model_cost: Object
    calls: list[TransformCall]
    model_info: NotRequired[Object]
    drop_params: NotRequired[bool]


class EventStream(TypedDict):
    function: Literal["event_stream"]
    chunks: list[str]
    response_headers: NotRequired[dict[str, str] | None]


class ErrorClass(TypedDict):
    function: Literal["error_class"]
    message: str
    status_code: int
    headers: NotRequired[dict[str, str]]


class Constructor(TypedDict):
    function: Literal["constructor"]
    kwargs: NotRequired[Object]


Input: TypeAlias = InvokeUrl | Transform | EventStream | ErrorClass | Constructor


class ErrorView(Protocol):
    status_code: int
    message: str
    headers: Object | None


class LoggingStub:
    pass


class StreamView(Protocol):
    def __aiter__(self) -> "StreamView": ...

    async def __anext__(self) -> bytes: ...

    async def aclose(self) -> None: ...


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


class SigningArgs(TypedDict):
    headers: dict[str, str]
    optional_params: Object
    request_data: Object
    api_base: str
    api_key: str | None
    model: str | None
    stream: bool | None
    fake_stream: bool | None


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

    def sign_request(
        self, **arguments: Unpack[SigningArgs]
    ) -> tuple[dict[str, str], bytes | None]: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> ErrorView: ...

    def get_async_streaming_response_iterator(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        response: httpx.Response,
        request_body: Object,
        logging: LoggingStub,
    ) -> StreamView: ...


VIEW: Final[TypeAdapter[MessagesView]] = TypeAdapter(MessagesView, config=ARBITRARY)


class Probe(MessagesConfig):
    def __init__(self) -> None:
        BaseAWSLLM.__init__(self)

    def view(self) -> MessagesView:
        return VIEW.validate_python(self)
