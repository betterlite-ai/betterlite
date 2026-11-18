import time
from functools import partial
from typing import Literal

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import LiteLLMBatch
from pydantic import JsonValue

from .api import AUTH_HEADERS, CONFIG, PARSE_PREFIX
from .models import REPLY, Auth, AuthScenario, Bodies, Create, Errors, ErrorScenario, ErrorView
from .wire import apply_environment, header_fields, new_logging


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
    return {"batch": REPLY.validate_json(batch.model_dump_json())}


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
    return {"batch": REPLY.validate_json(batch.model_dump_json())}


def error_class_value(item: ErrorScenario) -> JsonValue:
    headers: dict[str, str] | httpx.Headers
    if item.headers is not None:
        headers = dict(item.headers)
    elif item.headers_object is not None:
        headers = httpx.Headers(dict(item.headers_object))
    else:
        raise ValueError("an error scenario needs headers or headers_object")
    return ErrorView.of(CONFIG.get_error_class(item.message, item.status, headers))


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


def auth_of(headers: dict[str, str], expected: str | None) -> JsonValue:
    for name in AUTH_HEADERS:
        if name in headers:
            return {"name": name, "matches": headers[name] == expected}
    return {"name": None, "matches": expected is None}


def auth_scenario(scenario: AuthScenario) -> JsonValue:
    apply_environment(scenario.environment)
    headers: dict[str, str] = dict(scenario.caller_headers)
    try:
        CONFIG.validate_environment(
            headers=headers,
            model="",
            messages=[],
            optional_params={},
            litellm_params={"api_base": scenario.params_api_base},
            api_key=scenario.api_key,
            api_base=scenario.api_base,
        )
    except ValueError as error:
        raised: dict[str, JsonValue] = {"kind": "ValueError", "message": str(error)}
        return {"headers": None, "auth": None, "raised": raised}
    return {
        "headers": header_fields(headers, None),
        "auth": auth_of(headers, scenario.expected_auth),
        "raised": None,
    }


def auth_headers(payload: JsonValue) -> JsonValue:
    case = Auth.model_validate(payload)
    scenarios: list[JsonValue] = [auth_scenario(scenario) for scenario in case.scenarios]
    return {"scenarios": scenarios}
