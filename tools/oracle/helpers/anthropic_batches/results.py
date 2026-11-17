import time
from functools import partial

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import ModelResponse
from pydantic import JsonValue

from .api import CONFIG
from .models import Bodies
from .views import response_view
from .wire import new_logging


def result_value(body: str, logging_obj: Logging) -> JsonValue:
    response = ModelResponse(id="")
    try:
        transformed = CONFIG.transform_response(
            model="",
            raw_response=httpx.Response(200, content=body.encode()),
            model_response=response,
            logging_obj=logging_obj,
            request_data={},
            messages=[],
            optional_params={},
            litellm_params={},
            encoding=None,
        )
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"raised": True}
    return {"response": response_view(transformed)}


def result_response(payload: JsonValue) -> JsonValue:
    case = Bodies.model_validate(payload)
    logging_obj = new_logging(case.now)
    clock = time.time
    time.time = partial(float, case.now)
    try:
        values: list[JsonValue] = [result_value(body, logging_obj) for body in case.bodies]
    finally:
        time.time = clock
    return {"bodies": values}
