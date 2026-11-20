from typing import Final, Literal, Protocol, TypeAlias, TypedDict

from litellm.exceptions import UnsupportedParamsError
from litellm.llms.sail.chat.transformation import SailChatConfig
from pydantic import BaseModel, JsonValue, TypeAdapter

OBJECT = TypeAdapter(dict[str, JsonValue])


class WindowStep(TypedDict):
    service_tier: JsonValue
    drop_params: bool
    global_drop_params: bool


class WindowCall(TypedDict):
    function: Literal["window"]
    model: str
    steps: list[WindowStep]


class ChatRequestStep(TypedDict):
    request: dict[str, JsonValue]
    drop_params: bool
    global_drop_params: bool


class ChatRequestCall(TypedDict):
    function: Literal["chat_request"]
    model: str
    steps: list[ChatRequestStep]


class ExtraBodyStep(TypedDict):
    extra_body: dict[str, JsonValue]
    request_metadata: JsonValue
    drop_params: bool
    global_drop_params: bool


class ExtraBodyCall(TypedDict):
    function: Literal["extra_body"]
    model: str
    steps: list[ExtraBodyStep]


class ResponsesParamsStep(TypedDict):
    params: dict[str, JsonValue]
    drop_params: bool
    global_drop_params: bool


class ResponsesParamsCall(TypedDict):
    function: Literal["responses_params"]
    model: str
    steps: list[ResponsesParamsStep]


class SupportedStep(TypedDict):
    model: str
    chat_models: list[str]
    text_models: list[str]


class SupportedCall(TypedDict):
    function: Literal["supported"]
    steps: list[SupportedStep]


Input: TypeAlias = (
    WindowCall | ChatRequestCall | ExtraBodyCall | ResponsesParamsCall | SupportedCall
)
INPUT = TypeAdapter(Input)


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...


class Failure(BaseModel):
    status_code: int | None


# LiteLLM's exception class carries Any, so the catch names it through a typed alias
UNSUPPORTED: Final[type[Exception]] = UnsupportedParamsError

CHAT: ChatConfig = SailChatConfig()
