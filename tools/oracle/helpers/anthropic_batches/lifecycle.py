import time
from functools import partial
from typing import Literal

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import LiteLLMBatch
from pydantic import JsonValue

from .api import CONFIG, PARSE_PREFIX
from .models import Bodies, Create, ErrorScenario, ErrorView, Errors
from .views import batch_view
from .wire import apply_environment, new_logging


def transform_lifecycle(body: str, logging_obj: Logging) -> LiteLLMBatch:
    return CONFIG.transform_retrieve_batch_response(
        model=None,
        raw_response=httpx.Response(200, content=body.encode()),
        logging_obj=logging_obj,
        litellm_params={},
    )


def create_outcome(kind: Literal["request", "response"], logging_obj: Logging) -> JsonValue:
    try:
        if kind == "request":
            CONFIG.transform_create_batch_request("", {}, {}, {})
        else:
            CONFIG.transform_create_batch_response(None, httpx.Response(200), logging_obj, {})
    except NotImplementedError as error:
        return {"class": "NotImplementedError", "message": str(error)}
    raise ValueError(f"transform_create_batch_{kind} did not raise")


def create_requests(payload: JsonValue) -> JsonValue:
    case = Create.model_validate(payload)
    apply_environment(case.environment)
    logging_obj = new_logging(1_704_067_200)
    urls: list[JsonValue] = [
        CONFIG.get_complete_batch_url(base, None, "", {}, {}, {}) for base in case.url_bases
    ]
    return {
        "urls": urls,
        "create_request": create_outcome("request", logging_obj),
        "create_response": create_outcome("response", logging_obj),
    }


def lifecycle_value(body: str, logging_obj: Logging) -> JsonValue:
    try:
        batch = transform_lifecycle(body, logging_obj)
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"raised": True}
    return {"batch": batch_view(batch)}


def lifecycle(payload: JsonValue) -> JsonValue:
    case = Bodies.model_validate(payload)
    logging_obj = new_logging(case.now)
    clock = time.time
    time.time = partial(float, case.now)
    try:
        values: list[JsonValue] = [lifecycle_value(body, logging_obj) for body in case.bodies]
    finally:
        time.time = clock
    return {"batches": values}


def error_body_value(body: str, logging_obj: Logging) -> JsonValue:
    try:
        batch = transform_lifecycle(body, logging_obj)
    except ValueError as error:
        return {"prefix": str(error).startswith(PARSE_PREFIX)}
    except (AttributeError, KeyError, TypeError):
        return {"prefix": False}
    return {"batch": batch_view(batch)}


def error_class_view(item: ErrorScenario, headers: dict[str, str] | httpx.Headers) -> JsonValue:
    error = CONFIG.get_error_class(item.message, item.status, headers)
    view = ErrorView.model_validate(error, from_attributes=True)
    return {"status": view.status_code, "message": view.message, "headers": view.headers}


def error_class_value(item: ErrorScenario) -> JsonValue:
    if item.headers is not None:
        return error_class_view(item, dict(item.headers))
    if item.headers_object is not None:
        return error_class_view(item, httpx.Headers(dict(item.headers_object)))
    raise ValueError("an error scenario needs headers or headers_object")


def error_counts(payload: JsonValue) -> JsonValue:
    case = Errors.model_validate(payload)
    logging_obj = new_logging(case.now)
    clock = time.time
    time.time = partial(float, case.now)
    try:
        bodies: list[JsonValue] = [error_body_value(body, logging_obj) for body in case.bodies]
    finally:
        time.time = clock
    errors: list[JsonValue] = [error_class_value(item) for item in case.errors]
    return {"bodies": bodies, "errors": errors}
