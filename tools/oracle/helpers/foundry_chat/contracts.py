from typing import Final, NotRequired, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
from litellm.exceptions import UnsupportedParamsError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.chat.transformation import AzureAIStudioConfig
from litellm.types.utils import ModelResponse, ProviderField
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
Messages: TypeAlias = list[JsonObject]
Headers: TypeAlias = dict[str, str]
Environment: TypeAlias = dict[str, str]
AZURE_NAMES: Final[tuple[str, ...]] = (
    "AZURE_AI_API_BASE",
    "AZURE_AI_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "AZURE_API_KEY",
)
# the exception class carries Any, so the catch names it through a typed alias
UNSUPPORTED: Final[type[Exception]] = UnsupportedParamsError


class UrlCall(TypedDict):
    api_base: str | None
    api_version: str | None


class MapCall(TypedDict):
    model: str
    non_default: JsonObject
    optional: JsonObject
    drop_params: bool
    global_drop_params: bool


class RequestCall(TypedDict):
    model: str
    messages: Messages
    optional: JsonObject


class ResponseCall(TypedDict):
    model: str
    body: str


class ProviderCall(TypedDict):
    model: str
    api_base: str | None
    api_key: str | None
    environment: Environment


class AuthCall(TypedDict):
    api_base: str | None
    api_key: str | None
    headers: Headers
    litellm_params: JsonObject
    environment: Environment


class Input(TypedDict):
    model_cost: dict[str, JsonObject]
    chat_models: list[str]
    text_models: list[str]
    # LiteLLM's embedding models are a Final set, so the case names the Rust fixture's entries
    embedding_models: NotRequired[list[str]]
    urls: NotRequired[list[UrlCall]]
    supported: NotRequired[list[str]]
    maps: NotRequired[list[MapCall]]
    requests: NotRequired[list[RequestCall]]
    responses: NotRequired[list[ResponseCall]]
    providers: NotRequired[list[ProviderCall]]
    auths: NotRequired[list[AuthCall]]
    fields: NotRequired[bool]


class MessageRecord(TypedDict):
    role: str
    content: NotRequired[str | None]


class ChoiceRecord(TypedDict):
    index: int
    message: MessageRecord
    finish_reason: NotRequired[str | None]


class ResponseRecord(TypedDict):
    id: str
    object: str
    created: int
    model: NotRequired[str | None]
    system_fingerprint: NotRequired[str | None]
    usage: NotRequired[JsonObject | None]
    choices: list[ChoiceRecord]


# the message type carries Any, so the config enters through this JSON-facing signature
@runtime_checkable
class ChatConfig(Protocol):
    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: JsonObject,
        litellm_params: JsonObject,
    ) -> str: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: JsonObject,
        optional_params: JsonObject,
        model: str,
        *,
        drop_params: bool,
    ) -> JsonObject: ...

    def get_required_params(self) -> list[ProviderField]: ...

    def transform_request(
        self,
        model: str,
        messages: Messages,
        optional_params: JsonObject,
        litellm_params: JsonObject,
        headers: JsonObject,
    ) -> JsonObject: ...

    def validate_environment(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/chat/transformation.py#L123-L145
        self,
        headers: Headers,
        model: str,
        messages: Messages,
        optional_params: JsonObject,
        litellm_params: JsonObject,
        api_key: str | None,
        api_base: str | None,
    ) -> Headers: ...

    def transform_response(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/chat/transformation.py#L298-L325
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: Logging,
        request_data: JsonObject,
        messages: Messages,
        optional_params: JsonObject,
        litellm_params: JsonObject,
        encoding: None,
        api_key: str | None,
        *,
        json_mode: bool | None,
    ) -> ModelResponse: ...


class Failure(BaseModel):
    status_code: int | None


class ConfigBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: ChatConfig


CONFIG_VALUES: Final = {"config": AzureAIStudioConfig()}
CONFIG: Final[ChatConfig] = ConfigBoundary.model_validate(CONFIG_VALUES).config
RESPONSE: Final[TypeAdapter[ResponseRecord]] = TypeAdapter(ResponseRecord)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
