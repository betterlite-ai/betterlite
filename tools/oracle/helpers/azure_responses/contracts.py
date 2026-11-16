from collections.abc import Callable
from typing import Final, Literal, NotRequired, Protocol, Required, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.responses.o_series_transformation import (
    AzureOpenAIOSeriesResponsesAPIConfig as OSeriesResponses,
)
from litellm.types.llms.openai import ResponsesAPIOptionalRequestParams as RequestParams
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from openai.types.responses.response_input_param import ResponseInputParam
from openai.types.responses.response_text_config_param import ResponseTextConfigParam
from pydantic import BaseModel, JsonValue, TypeAdapter

Object: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]
Order: TypeAlias = Literal["asc", "desc"]
Endpoint: TypeAlias = Literal["get", "delete", "cancel", "list"]
Cancellation: TypeAlias = Callable[[httpx.Response, Logging], ResponsesAPIResponse]
Writer: TypeAlias = Callable[[str, str, GenericLiteLLMParams, Headers], tuple[str, Object]]
Listing: TypeAlias = Callable[
    [
        str,
        str,
        GenericLiteLLMParams,
        Headers,
        str | None,
        str | None,
        list[str] | None,
        int,
        Order,
    ],
    tuple[str, Object],
]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ResponsesConfig(Protocol):
    @property
    def custom_llm_provider(self) -> LlmProviders: ...

    @property
    def transform_get_response_api_request(self) -> Writer: ...  # ast-grep-ignore

    @property
    def transform_delete_response_api_request(self) -> Writer: ...  # ast-grep-ignore

    @property
    def transform_cancel_response_api_request(self) -> Writer: ...  # ast-grep-ignore

    @property
    def transform_list_input_items_request(self) -> Listing: ...  # ast-grep-ignore

    @property
    def transform_cancel_response_api_response(self) -> Cancellation: ...  # ast-grep-ignore

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self, response_api_optional_params: RequestParams, model: str, *, drop_params: bool
    ) -> Object: ...

    def validate_environment(
        self, headers: Headers, model: str, litellm_params: GenericLiteLLMParams | None
    ) -> Headers: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def get_websocket_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def transform_responses_api_request(  # ast-grep-ignore
        self,
        model: str,
        input_value: str | ResponseInputParam,
        params: Object,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
        /,
    ) -> Object: ...

    def supports_native_websocket(self) -> bool: ...

    def model_in_websocket_url(self) -> bool: ...


class OSeriesConfig(ResponsesConfig, Protocol):
    def is_o_series_model(self, model: str) -> bool: ...


class PromptView(TypedDict, total=False):
    id: Required[str]
    variables: dict[str, JsonValue] | None
    version: str | None


class Params(RequestParams, total=False):
    metadata: dict[str, JsonValue] | None
    prompt: PromptView | None
    text: ResponseTextConfigParam | None


class Install(TypedDict):
    model_cost: dict[str, dict[str, JsonValue]]
    chat_models: list[str]
    text_models: list[str]
    drop_params: bool


class UrlStep(TypedDict):
    api_base: str | None
    api_version: str | None
    headers: Headers
    api_key: str | None


class UrlCall(TypedDict):
    function: Literal["base_and_api_version"]
    default_api_version: str
    steps: list[UrlStep]


class TransformStep(TypedDict):
    model: str
    input: str | ResponseInputParam
    params: NotRequired[Object]


class TransformCall(TypedDict):
    function: Literal["strip_model_prefix", "reasoning_null_fields"]
    steps: list[TransformStep]


class WebsocketStep(TypedDict):
    api_base: str | None


class WebsocketCall(TypedDict):
    function: Literal["websocket_model_query"]
    steps: list[WebsocketStep]


class ResourceStep(TypedDict):
    endpoint: Endpoint
    api_base: str
    response_id: str
    after: NotRequired[str | None]
    before: NotRequired[str | None]
    include: NotRequired[list[str] | None]
    limit: NotRequired[int | None]
    order: NotRequired[Order | None]


class ResourceCall(TypedDict):
    function: Literal["resource_id_before_query"]
    steps: list[ResourceStep]


class ReplyStep(TypedDict):
    status: int
    body: str


class ReplyCall(TypedDict):
    function: Literal["cancel_response"]
    steps: list[ReplyStep]


class MapStep(TypedDict):
    model: str
    params: NotRequired[Params]
    drop_params: bool
    global_drop_params: bool


class CapabilityCall(TypedDict):
    function: Literal["azure_responses_capabilities", "o_series_options"]
    install: Install
    models: list[str]
    map_steps: list[MapStep]
    o_series_steps: list[MapStep]


Call: TypeAlias = (
    UrlCall | TransformCall | WebsocketCall | ResourceCall | ReplyCall | CapabilityCall
)


class Failure(BaseModel):
    status_code: int | None = None


def azure_config() -> ResponsesConfig:
    found = ProviderConfigManager.get_provider_responses_api_config(provider=LlmProviders.AZURE)
    if found is None:
        raise ValueError("the azure responses config is not registered")
    return found


AZURE: Final[ResponsesConfig] = azure_config()
OSERIES: Final[OSeriesConfig] = OSeriesResponses.__new__(OSeriesResponses)
