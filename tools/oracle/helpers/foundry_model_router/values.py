import os
from collections.abc import Generator
from contextlib import contextmanager
from datetime import UTC, datetime

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.common_utils import AzureFoundryModelInfo
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.types.utils import ModelResponse
from pydantic import JsonValue

from .contracts import (
    CONFIG,
    INFO,
    RESPONSE,
    VERSION_NAMES,
    CatalogCall,
    JsonObject,
    PropertyCall,
    ReaderCall,
    ResponseCall,
    StripCall,
    VersionCall,
)


def strip_value(call: StripCall) -> JsonObject:
    model = call["model"]
    body = CONFIG.transform_request(model, [], {}, {}, {})
    return {"stripped": AzureFoundryModelInfo.strip_model_router_prefix(model), "body": body}


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
    )
    record = RESPONSE.validate_json(response.model_dump_json())
    hidden = get_hidden_params_dict(response)
    return {
        "model": record.get("model"),
        "selected": AzureFoundryModelInfo.get_model_router_selected_model(hidden),
    }


def reader_value(call: ReaderCall) -> JsonObject:
    hidden = call.get("hidden_params")
    return {
        "selected": AzureFoundryModelInfo.get_model_router_selected_model(hidden),
        "is_call": AzureFoundryModelInfo.is_model_router_call(
            model=call.get("model"), hidden_params=hidden
        ),
    }


def catalog_value(call: CatalogCall) -> JsonObject:
    models = AzureFoundryModelInfo().get_models(
        api_key=call.get("api_key"), api_base=call.get("api_base")
    )
    listed: list[JsonValue] = []
    listed.extend(models)
    return {"models": listed}


@contextmanager
def version_context(
    global_api_version: str | None, environment: dict[str, str]
) -> Generator[None, None, None]:
    for name in VERSION_NAMES:
        os.environ.pop(name, None)
    os.environ.update(environment)
    previous = litellm.api_version
    litellm.api_version = global_api_version
    try:
        yield
    finally:
        litellm.api_version = previous


def version_value(call: VersionCall) -> JsonObject:
    with version_context(call.get("global_api_version"), call["environment"]):
        api_version = AzureFoundryModelInfo.get_api_version(call.get("api_version"))
    return {"api_version": api_version}


def property_value(call: PropertyCall) -> JsonObject:
    with version_context(call.get("global_api_version"), call["environment"]):
        api_version = INFO.api_version
    return {"api_version": api_version}


def validate_value() -> JsonObject:
    try:
        INFO.validate_environment({}, "fixture-model", [], {}, {}, None, None)
    except NotImplementedError as error:
        return {"kind": type(error).__name__, "message": str(error)}
    return {"kind": None, "message": None}
