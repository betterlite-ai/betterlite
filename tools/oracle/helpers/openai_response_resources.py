import time
from collections.abc import Callable
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, Protocol, TypeAlias

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.responses.main import DeleteResponseResult
from litellm.types.router import GenericLiteLLMParams as Params
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import BaseModel, ConfigDict, Field, JsonValue
from typing_extensions import TypedDict

litellm.suppress_debug_info = True

Provider: TypeAlias = Literal["openai", "azure", "hosted_vllm"]
Endpoint: TypeAlias = Literal["delete", "get", "list", "cancel", "compact"]
Headers: TypeAlias = dict[str, JsonValue]
Item: TypeAlias = Callable[[str, str, Params, Headers], tuple[str, Headers]]
Listing: TypeAlias = Callable[
    [
        str, str, Params, Headers, str | None, str | None, list[str] | None, int,
        Literal["asc", "desc"],
    ],
    tuple[str, Headers],
]
Compact: TypeAlias = Callable[
    [str, str | ResponseInputParam, Headers, str, Params, Headers], tuple[str, Headers]
]
Reply: TypeAlias = Callable[[httpx.Response, Logging], ResponsesAPIResponse]
Page: TypeAlias = Callable[[httpx.Response, Logging], Headers]
DeleteReply: TypeAlias = Callable[[httpx.Response, Logging], DeleteResponseResult]

PROVIDERS: Final = {
    "openai": LlmProviders.OPENAI,
    "azure": LlmProviders.AZURE,
    "hosted_vllm": LlmProviders.HOSTED_VLLM,
}


class Probe(TypedDict):
    endpoint: NotRequired[Endpoint]
    response_id: NotRequired[str]
    api_base: NotRequired[str]
    body: NotRequired[str]
    status: NotRequired[int]
    headers: NotRequired[dict[str, str]]
    model: NotRequired[str]
    input: NotRequired[str | ResponseInputParam]
    params: NotRequired[Headers]
    provider: NotRequired[Provider]
    model_info: NotRequired[Headers | None]
    after: NotRequired[str | None]
    before: NotRequired[str | None]
    include: NotRequired[list[str] | None]
    limit: NotRequired[int]
    order: NotRequired[Literal["asc", "desc"]]


# LiteLLM's request conversions type their data as a bare dict, so the callables fix the shape
class ResponsesConfig(Protocol):
    @property
    def transform_delete_response_api_request(self) -> Item: ...  # ast-grep-ignore

    @property
    def transform_get_response_api_request(self) -> Item: ...  # ast-grep-ignore

    @property
    def transform_cancel_response_api_request(self) -> Item: ...  # ast-grep-ignore

    @property
    def transform_list_input_items_request(self) -> Listing: ...  # ast-grep-ignore

    @property
    def transform_compact_response_api_request(self) -> Compact: ...  # ast-grep-ignore

    @property
    def transform_list_input_items_response(self) -> Page: ...  # ast-grep-ignore

    @property
    def transform_get_response_api_response(self) -> Reply: ...  # ast-grep-ignore

    @property
    def transform_cancel_response_api_response(self) -> Reply: ...  # ast-grep-ignore

    @property
    def transform_compact_response_api_response(self) -> Reply: ...  # ast-grep-ignore

    @property
    def transform_delete_response_api_response(self) -> DeleteReply: ...  # ast-grep-ignore


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int | None = None


class HiddenParams(TypedDict):
    headers: Headers
    additional_headers: Headers


class Hidden(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    params: HiddenParams = Field(alias="_hidden_params")


class PostCall(TypedDict):
    original_response: str
    additional_args: Headers


class Recorded(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: PostCall


def config_for(provider: Provider | None) -> ResponsesConfig:
    found = ProviderConfigManager.get_provider_responses_api_config(
        provider=PROVIDERS["openai" if provider is None else provider]
    )
    if found is None:
        raise ValueError("the responses config is not registered")
    return found


def logging_object() -> Logging:
    return Logging(
        model="gpt-4o", messages=None, stream=False, call_type="responses",
        start_time=datetime(2026, 1, 1, tzinfo=UTC), litellm_call_id="call-1", function_id="fn-1",
    )


def required(value: str | None, name: str) -> str:
    if value is None:
        raise ValueError(f"the probe needs {name}")
    return value


def raw_response(probe: Probe) -> httpx.Response:
    status = probe.get("status")
    if status is None:
        raise ValueError("a reply probe needs status")
    request = httpx.Request("POST", "https://api.openai.com/v1/responses")
    given = probe.get("headers")
    response = httpx.Response(
        status,
        stream=httpx.ByteStream(required(probe.get("body"), "body").encode()),
        headers={} if given is None else given,
        request=request,
    )
    response.read()
    return response


def typed_value(reply: ResponsesAPIResponse) -> JsonValue:
    hidden = Hidden.model_validate(reply)
    return {
        "id": reply.id,
        "created_at": reply.created_at,
        "status": reply.status,
        "model": reply.model,
        "headers": hidden.params["headers"],
        "additional_headers": hidden.params["additional_headers"],
    }


def compact_value(
    config: ResponsesConfig, response: httpx.Response, logging_obj: Logging
) -> JsonValue:
    started = int(time.time())
    reply = config.transform_compact_response_api_response(response, logging_obj)
    finished = int(time.time())
    hidden = Hidden.model_validate(reply)
    recorded = Recorded.model_validate(logging_obj).model_call_details
    return {
        "id": reply.id,
        "created_at": "now" if started <= reply.created_at <= finished else reply.created_at,
        "headers": hidden.params["headers"],
        "additional_headers": hidden.params["additional_headers"],
        "post_call": {
            "original_response": recorded["original_response"],
            "additional_args": recorded["additional_args"],
        },
    }


def request_url(
    config: ResponsesConfig, endpoint: Endpoint, probe: Probe, api_base: str
) -> tuple[str, Headers]:
    litellm_params = Params(model_info=probe.get("model_info"))
    params = probe.get("params")
    if endpoint == "compact":
        input_value = probe.get("input")
        if input_value is None:
            raise ValueError("a compact request probe needs input")
        return config.transform_compact_response_api_request(
            required(probe.get("model"), "model"), input_value,
            {} if params is None else params, api_base, litellm_params, {},
        )
    response_id = required(probe.get("response_id"), "response_id")
    if endpoint == "list":
        limit = probe.get("limit")
        order = probe.get("order")
        return config.transform_list_input_items_request(
            response_id, api_base, litellm_params, {}, probe.get("after"), probe.get("before"),
            probe.get("include"), 20 if limit is None else limit,
            "desc" if order is None else order,
        )
    if endpoint == "delete":
        return config.transform_delete_response_api_request(
            response_id, api_base, litellm_params, {}
        )
    if endpoint == "get":
        return config.transform_get_response_api_request(response_id, api_base, litellm_params, {})
    if endpoint == "cancel":
        return config.transform_cancel_response_api_request(
            response_id, api_base, litellm_params, {}
        )
    raise ValueError(f"{endpoint} has no request")


def request_value(probe: Probe) -> JsonValue:
    endpoint = probe.get("endpoint")
    if endpoint is None:
        raise ValueError("the probe needs endpoint")
    api_base = required(probe.get("api_base"), "api_base")
    provider = probe.get("provider") if endpoint == "compact" else None
    try:
        url, data = request_url(config_for(provider), endpoint, probe, api_base)
    except (ValueError, TypeError, KeyError):
        return {"error": {"status": None}}
    except Exception as error:
        failure = Failure.model_validate(error)
        if failure.status_code is None:
            raise
        return {"error": {"status": failure.status_code}}
    return {"url": url, "data": data}


def reply_value(
    config: ResponsesConfig, endpoint: Endpoint, response: httpx.Response, logging_obj: Logging
) -> JsonValue:
    if endpoint == "delete":
        deleted = config.transform_delete_response_api_response(response, logging_obj)
        return {"id": deleted.id, "object": deleted.object, "deleted": deleted.deleted}
    if endpoint == "get":
        return typed_value(config.transform_get_response_api_response(response, logging_obj))
    if endpoint == "cancel":
        return typed_value(config.transform_cancel_response_api_response(response, logging_obj))
    if endpoint == "list":
        return config.transform_list_input_items_response(response, logging_obj)
    return compact_value(config, response, logging_obj)


def response_value(probe: Probe) -> JsonValue:
    endpoint = probe.get("endpoint")
    if endpoint is None:
        raise ValueError("the probe needs endpoint")
    response = raw_response(probe)
    logging_obj = logging_object()
    try:
        return reply_value(config_for(None), endpoint, response, logging_obj)
    except (ValueError, TypeError, KeyError):
        return {"error": {"status": None}}
    except Exception as error:
        failure = Failure.model_validate(error)
        if failure.status_code is None:
            raise
        return {"error": {"status": failure.status_code}}


HANDLERS: Final[dict[str, Callable[[Probe], JsonValue]]] = {
    "encoded_response_id": request_value,
    "delete_result": response_value,
    "get_strict_response": response_value,
    "list_default_limit_order": request_value,
    "include_comma_query": request_value,
    "cancel_response": response_value,
    "compact_keeps_query": request_value,
    "compact_replay_input": request_value,
    "compact_lax_response": response_value,
    "resource_json_error_status": response_value,
}


def run(input_value: dict[str, dict[str, list[Probe]]]) -> JsonValue:
    if len(input_value) != 1:
        raise ValueError("expected one openai_response_resources case")
    name = next(iter(input_value))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown openai_response_resources case: {name}")
    return [handler(probe) for probe in input_value[name]["probes"]]
