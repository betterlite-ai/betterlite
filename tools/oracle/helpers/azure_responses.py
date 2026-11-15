import io
from collections.abc import Callable
from contextlib import redirect_stdout
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.exceptions import UnsupportedParamsError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.chat.gpt_transformation import AzureOpenAIError
from litellm.llms.azure.responses.o_series_transformation import (
    AzureOpenAIOSeriesResponsesAPIConfig,
)
from litellm.types.llms.openai import (
    ResponsesAPIOptionalRequestParams,
    ResponsesAPIResponse,
)
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import BaseModel, ConfigDict, JsonValue

litellm.suppress_debug_info = True

Object: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]
Function: TypeAlias = Literal[
    "base_and_api_version",
    "strip_model_prefix",
    "o_series_options",
    "reasoning_null_fields",
    "websocket_model_query",
    "resource_id_before_query",
    "cancel_response",
    "azure_responses_capabilities",
]
Cancellation: TypeAlias = Callable[[httpx.Response, Logging], ResponsesAPIResponse]
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
        Literal["asc", "desc"],
    ],
    tuple[str, Object],
]
ORDER: Final = ("model", "input", "tools")
UNSUPPORTED: Final[type[Exception]] = UnsupportedParamsError


class ResponsesConfig(Protocol):
    @property
    def custom_llm_provider(self) -> LlmProviders: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        response_api_optional_params: ResponsesAPIOptionalRequestParams,
        model: str,
        *,
        drop_params: bool,
    ) -> Object: ...

    def validate_environment(
        self, headers: Headers, model: str, litellm_params: GenericLiteLLMParams | None
    ) -> Headers: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def transform_responses_api_request(
        self,
        model: str,
        input_value: str | ResponseInputParam,
        params: Object,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
        /,
    ) -> Object: ...

    def get_websocket_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def supports_native_websocket(self) -> bool: ...

    def model_in_websocket_url(self) -> bool: ...

    def transform_get_response_api_request(
        self,
        response_id: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, Object]: ...

    def transform_delete_response_api_request(
        self,
        response_id: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, Object]: ...

    def transform_cancel_response_api_request(
        self,
        response_id: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, Object]: ...

    @property
    def transform_list_input_items_request(self) -> Listing: ...  # ast-grep-ignore

    @property
    def transform_cancel_response_api_response(self) -> Cancellation: ...  # ast-grep-ignore


class OSeriesConfig(ResponsesConfig, Protocol):
    def is_o_series_model(self, model: str) -> bool: ...


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


class TransformStep(TypedDict):
    model: str
    input: str | ResponseInputParam
    params: NotRequired[Object]


class WebsocketStep(TypedDict):
    api_base: str | None


class ResourceStep(TypedDict):
    endpoint: Literal["get", "delete", "cancel", "list"]
    api_base: str
    response_id: str
    after: NotRequired[str | None]
    before: NotRequired[str | None]
    include: NotRequired[list[str] | None]
    limit: NotRequired[int | None]
    order: NotRequired[Literal["asc", "desc"] | None]


class ReplyStep(TypedDict):
    status: int
    body: str


class MapStep(TypedDict):
    model: str
    params: ResponsesAPIOptionalRequestParams
    drop_params: bool
    global_drop_params: bool


class OSeriesStep(TypedDict):
    model: str
    params: ResponsesAPIOptionalRequestParams
    drop_params: bool


class Call(TypedDict):
    function: Function
    default_api_version: NotRequired[str]
    install: NotRequired[Install]
    urls: NotRequired[list[UrlStep]]
    transforms: NotRequired[list[TransformStep]]
    websockets: NotRequired[list[WebsocketStep]]
    resources: NotRequired[list[ResourceStep]]
    replies: NotRequired[list[ReplyStep]]
    models: NotRequired[list[str]]
    map_steps: NotRequired[list[MapStep]]
    o_series_steps: NotRequired[list[OSeriesStep]]


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int | None = None


def status_of(error: Exception) -> int | None:
    return Failure.model_validate(error).status_code


def azure_config() -> ResponsesConfig:
    found = ProviderConfigManager.get_provider_responses_api_config(
        provider=LlmProviders.AZURE
    )
    if found is None:
        raise ValueError("the azure responses config is not registered")
    return found


def o_series_config() -> OSeriesConfig:
    return AzureOpenAIOSeriesResponsesAPIConfig()


def install(source: Install) -> None:
    litellm.model_cost = source["model_cost"]
    litellm.open_ai_chat_completion_models = set(source["chat_models"])
    litellm.open_ai_text_completion_models = set(source["text_models"])
    litellm.model_list_set = set(source["chat_models"]) | set(source["text_models"])
    litellm.drop_params = source["drop_params"]


def logging_object() -> Logging:
    return Logging(
        model="gpt-4o",
        messages=None,
        stream=False,
        call_type="responses",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def url_row(step: UrlStep) -> JsonValue:
    config = azure_config()
    try:
        url = config.get_complete_url(
            step["api_base"], {"api_version": step["api_version"]}
        )
        headers = config.validate_environment(
            dict(step["headers"]), "gpt-5", GenericLiteLLMParams(api_key=step["api_key"])
        )
        return {"ok": True, "url": url, "headers": headers, "status": None}
    except ValueError as error:
        return {"ok": False, "url": None, "headers": None, "status": status_of(error)}


def transform_row(step: TransformStep) -> JsonValue:
    body = azure_config().transform_responses_api_request(
        step["model"], step["input"], step.get("params") or {}, GenericLiteLLMParams(), {}
    )
    projected: Object = {}
    for name in ORDER:
        if name in body:
            projected[name] = body[name]
    return projected


def websocket_step_row(step: WebsocketStep) -> JsonValue:
    try:
        url = azure_config().get_websocket_url(step["api_base"], {})
        return {"ok": True, "url": url, "status": None}
    except ValueError as error:
        return {"ok": False, "url": None, "status": status_of(error)}


def websocket_report(call: Call) -> JsonValue:
    config = azure_config()
    return {
        "supports_native_websocket": config.supports_native_websocket(),
        "model_in_websocket_url": config.model_in_websocket_url(),
        "steps": [websocket_step_row(step) for step in call.get("websockets") or []],
    }


def resource_request(
    step: ResourceStep, litellm_params: GenericLiteLLMParams
) -> tuple[str, Object]:
    config = azure_config()
    if step["endpoint"] == "list":
        limit = step.get("limit")
        order = step.get("order")
        return config.transform_list_input_items_request(
            step["response_id"],
            step["api_base"],
            litellm_params,
            {},
            step.get("after"),
            step.get("before"),
            step.get("include"),
            20 if limit is None else limit,
            "desc" if order is None else order,
        )
    if step["endpoint"] == "get":
        return config.transform_get_response_api_request(
            step["response_id"], step["api_base"], litellm_params, {}
        )
    if step["endpoint"] == "delete":
        return config.transform_delete_response_api_request(
            step["response_id"], step["api_base"], litellm_params, {}
        )
    return config.transform_cancel_response_api_request(
        step["response_id"], step["api_base"], litellm_params, {}
    )


def resource_row(step: ResourceStep) -> JsonValue:
    try:
        url, data = resource_request(step, GenericLiteLLMParams())
        return {"ok": True, "url": url, "data": data, "status": None}
    except ValueError as error:
        return {"ok": False, "url": None, "data": None, "status": status_of(error)}


def reply_row(step: ReplyStep) -> JsonValue:
    response = httpx.Response(
        step["status"],
        stream=httpx.ByteStream(step["body"].encode()),
        request=httpx.Request("POST", "https://example.invalid/cancel"),
    )
    response.read()
    try:
        reply = azure_config().transform_cancel_response_api_response(
            response, logging_object()
        )
        return {
            "ok": True,
            "value": {
                "id": reply.id,
                "created_at": reply.created_at,
                "status": reply.status,
                "model": reply.model,
            },
            "status": None,
        }
    except (ValueError, AzureOpenAIError) as error:
        return {"ok": False, "value": None, "status": status_of(error)}


def map_value(step: MapStep) -> JsonValue:
    litellm.drop_params = step["global_drop_params"]
    try:
        mapped = azure_config().map_openai_params(
            step["params"], step["model"], drop_params=step["drop_params"]
        )
    except UNSUPPORTED as error:
        return {"ok": False, "value": None, "status": status_of(error)}
    return {"ok": True, "value": mapped, "status": None}


def o_series_value(step: OSeriesStep) -> JsonValue:
    mapped = o_series_config().map_openai_params(
        step["params"], step["model"], drop_params=step["drop_params"]
    )
    return {"ok": True, "value": mapped, "status": None}


def capability_report(call: Call) -> JsonValue:
    source = call.get("install")
    if source is None:
        raise ValueError("a capability call needs install")
    install(source)
    config = azure_config()
    o_series = o_series_config()
    supported: list[JsonValue] = [*sorted(config.get_supported_openai_params("gpt-5"))]
    o_series_supported: list[JsonValue] = [
        *sorted(o_series.get_supported_openai_params("o3"))
    ]
    models: list[JsonValue] = []
    for model in call.get("models") or []:
        models.append(
            {
                "model": model,
                "supported": [*sorted(config.get_supported_openai_params(model))],
                "o_series": o_series.is_o_series_model(model),
            }
        )
    return {
        "provider": config.custom_llm_provider.value,
        "supported": supported,
        "o_series_supported": o_series_supported,
        "models": models,
        "map_steps": [map_value(step) for step in call.get("map_steps") or []],
        "o_series_steps": [
            o_series_value(step) for step in call.get("o_series_steps") or []
        ],
    }


def call_value(call: Call) -> JsonValue:
    function = call["function"]
    if function == "base_and_api_version":
        return {"steps": [url_row(step) for step in call.get("urls") or []]}
    if function in ("strip_model_prefix", "reasoning_null_fields"):
        return {"steps": [transform_row(step) for step in call.get("transforms") or []]}
    if function == "websocket_model_query":
        return websocket_report(call)
    if function == "resource_id_before_query":
        return {"steps": [resource_row(step) for step in call.get("resources") or []]}
    if function == "cancel_response":
        return {"steps": [reply_row(step) for step in call.get("replies") or []]}
    return capability_report(call)


def run(input_value: Call) -> JsonValue:
    with redirect_stdout(io.StringIO()):
        return call_value(input_value)
