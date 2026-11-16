from datetime import UTC, datetime
from typing import Final

import httpx
import litellm
from litellm.exceptions import UnsupportedParamsError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue

from .contracts import (
    AZURE,
    JSON,
    OSERIES,
    CapabilityCall,
    Failure,
    Install,
    MapStep,
    Object,
    Params,
    ReplyStep,
    ResourceStep,
    TransformStep,
    UrlStep,
    WebsocketCall,
    WebsocketStep,
)

UNSUPPORTED: Final[type[Exception]] = UnsupportedParamsError
CANCEL_ERROR: Final[type[Exception]] = AzureOpenAIError
PROJECTED: Final[tuple[str, ...]] = ("model", "input", "tools")
LIMIT: Final[int] = 20


def install(source: Install) -> None:
    litellm.model_cost = source["model_cost"]
    litellm.open_ai_chat_completion_models = set(source["chat_models"])
    litellm.open_ai_text_completion_models = set(source["text_models"])
    models = set(source["chat_models"]) | set(source["text_models"])
    litellm.model_list_set = models
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
    try:
        url = AZURE.get_complete_url(step["api_base"], {"api_version": step["api_version"]})
        headers = AZURE.validate_environment(
            step["headers"], "gpt-5", GenericLiteLLMParams(api_key=step["api_key"])
        )
    except ValueError:
        return {"ok": False, "url": None, "headers": None, "status": None}
    return {"ok": True, "url": url, "headers": JSON.validate_python(headers), "status": None}


def transform_row(step: TransformStep) -> JsonValue:
    params = step.get("params")
    given: Object = {} if params is None else params
    body = AZURE.transform_responses_api_request(
        step["model"], step["input"], given, GenericLiteLLMParams(), {}
    )
    projected: Object = {}
    for name in PROJECTED:
        if name in body:
            projected[name] = body[name]
    return projected


def websocket_row(step: WebsocketStep) -> JsonValue:
    try:
        url = AZURE.get_websocket_url(step["api_base"], {})
    except (ValueError, httpx.InvalidURL):
        return {"ok": False, "url": None, "status": None}
    return {"ok": True, "url": url, "status": None}


def websocket_report(call: WebsocketCall) -> JsonValue:
    return {
        "supports_native_websocket": AZURE.supports_native_websocket(),
        "model_in_websocket_url": AZURE.model_in_websocket_url(),
        "steps": [websocket_row(step) for step in call["steps"]],
    }


def list_request(step: ResourceStep) -> tuple[str, Object]:
    limit = step.get("limit")
    order = step.get("order")
    return AZURE.transform_list_input_items_request(
        step["response_id"],
        step["api_base"],
        GenericLiteLLMParams(),
        {},
        step.get("after"),
        step.get("before"),
        step.get("include"),
        LIMIT if limit is None else limit,
        "desc" if order is None else order,
    )


def sent_request(step: ResourceStep) -> tuple[str, Object]:
    params = GenericLiteLLMParams()
    endpoint = step["endpoint"]
    if endpoint == "list":
        return list_request(step)
    if endpoint == "get":
        return AZURE.transform_get_response_api_request(
            step["response_id"], step["api_base"], params, {}
        )
    if endpoint == "delete":
        return AZURE.transform_delete_response_api_request(
            step["response_id"], step["api_base"], params, {}
        )
    return AZURE.transform_cancel_response_api_request(
        step["response_id"], step["api_base"], params, {}
    )


def resource_row(step: ResourceStep) -> JsonValue:
    try:
        url, data = sent_request(step)
    except ValueError:
        return {"ok": False, "url": None, "data": None, "status": None}
    return {"ok": True, "url": url, "data": data, "status": None}


def reply_row(step: ReplyStep) -> JsonValue:
    request = httpx.Request("POST", "https://example.invalid/cancel")
    response = httpx.Response(step["status"], content=step["body"].encode(), request=request)
    try:
        reply = AZURE.transform_cancel_response_api_response(response, logging_object())
    except ValueError:
        return {"ok": False, "value": None, "status": None}
    except CANCEL_ERROR as error:
        failure = Failure.model_validate(error, from_attributes=True)
        return {"ok": False, "value": None, "status": failure.status_code}
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


def map_row(step: MapStep) -> JsonValue:
    litellm.drop_params = step["global_drop_params"]
    params = step.get("params")
    given: Params = Params() if params is None else params
    try:
        mapped = AZURE.map_openai_params(given, step["model"], drop_params=step["drop_params"])
    except UNSUPPORTED as error:
        failure = Failure.model_validate(error, from_attributes=True)
        return {"ok": False, "value": None, "status": failure.status_code}
    return {"ok": True, "value": mapped, "status": None}


def o_series_row(step: MapStep) -> JsonValue:
    params = step.get("params")
    given: Params = Params() if params is None else params
    mapped = OSERIES.map_openai_params(given, step["model"], drop_params=step["drop_params"])
    return {"ok": True, "value": mapped, "status": None}


def model_row(model: str) -> JsonValue:
    return {
        "model": model,
        "supported": [*sorted(AZURE.get_supported_openai_params(model))],
        "o_series": OSERIES.is_o_series_model(model),
    }


def capability_report(call: CapabilityCall) -> JsonValue:
    install(call["install"])
    return {
        "provider": AZURE.custom_llm_provider.value,
        "supported": [*sorted(AZURE.get_supported_openai_params("gpt-5"))],
        "o_series_supported": [*sorted(OSERIES.get_supported_openai_params("o3"))],
        "models": [model_row(model) for model in call["models"]],
        "map_steps": [map_row(step) for step in call["map_steps"]],
        "o_series_steps": [o_series_row(step) for step in call["o_series_steps"]],
    }
