import os
from datetime import UTC, datetime

import httpx
import litellm
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import ModelResponse
from pydantic import JsonValue

from .contracts import (
    AZURE_NAMES,
    CONFIG,
    JSON,
    RESPONSE,
    UNSUPPORTED,
    AuthCall,
    Failure,
    Headers,
    JsonObject,
    MapCall,
    ProviderCall,
    RequestCall,
    ResponseCall,
    UrlCall,
)


def apply_environment(environment: Headers) -> None:
    for name in AZURE_NAMES:
        os.environ.pop(name, None)
    os.environ.update(environment)


def url_value(call: UrlCall) -> JsonObject:
    params: JsonObject = {"api_version": call["api_version"]}
    try:
        url = CONFIG.get_complete_url(call["api_base"], None, "fixture-model", {}, params)
    except ValueError:
        return {"error": {"kind": "ValueError"}}
    return {"value": url}


def map_value(call: MapCall) -> JsonObject:
    litellm.drop_params = call["global_drop_params"]
    optional: JsonObject = dict(call["optional"])
    try:
        mapped = CONFIG.map_openai_params(
            dict(call["non_default"]), optional, call["model"], drop_params=call["drop_params"]
        )
    except UNSUPPORTED as error:
        failure = Failure.model_validate(error, from_attributes=True)
        return {"ok": False, "value": None, "status": failure.status_code}
    return {"ok": True, "value": mapped, "status": None}


def request_value(call: RequestCall) -> JsonObject:
    body = CONFIG.transform_request(
        call["model"], call["messages"], dict(call["optional"]), {}, {}
    )
    return {"ok": True, "value": body}


def response_value(call: ResponseCall) -> JsonObject:
    logging_obj = Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    response = CONFIG.transform_response(
        call["model"],
        httpx.Response(200, content=call["body"].encode()),
        ModelResponse(),
        logging_obj,
        {},
        [],
        {},
        {},
        None,
        None,
        json_mode=None,
    )
    record = RESPONSE.validate_json(response.model_dump_json())
    usage = JSON.validate_python(record.get("usage"))
    choices: list[JsonValue] = [
        {
            "index": choice["index"],
            "message": {
                "role": choice["message"]["role"],
                "content": choice["message"].get("content"),
            },
            "finish_reason": choice.get("finish_reason"),
        }
        for choice in record["choices"]
    ]
    return {
        "id": record["id"],
        "object": record["object"],
        "created": record["created"],
        "model": record.get("model"),
        "system_fingerprint": record.get("system_fingerprint"),
        "usage": usage,
        "choices": choices,
    }


def provider_value(call: ProviderCall) -> JsonObject:
    apply_environment(call["environment"])
    _, provider, key, base = get_llm_provider(
        model=call["model"],
        custom_llm_provider="azure_ai",
        api_base=call["api_base"],
        api_key=call["api_key"],
    )
    return {"provider": provider, "api_base": base, "api_key": key}


def auth_value(call: AuthCall) -> JsonValue:
    apply_environment(call["environment"])
    headers = CONFIG.validate_environment(
        dict(call["headers"]),
        "fixture-model",
        [],
        {},
        call["litellm_params"],
        call["api_key"],
        call["api_base"],
    )
    return JSON.validate_python(headers)


def fields_value() -> JsonValue:
    return JSON.validate_python(CONFIG.get_required_params())
