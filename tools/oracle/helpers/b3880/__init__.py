from collections.abc import Callable
from typing import Final

import litellm
from pydantic import JsonValue

from .answers import (
    class_name_answer,
    exception_provider_answer,
    invalid_key_answer,
    rate_limit_answer,
    request_provider_answer,
    requested_model_answer,
    skip_answer,
    status_code_answer,
)
from .deployment import deployment_answer
from .values import Function, Input

# get_llm_provider prints its provider list to stdout on an unknown model, and the helper's stdout is one JSON document
litellm.suppress_debug_info = True

HANDLERS: Final[dict[Function, Callable[[Input], JsonValue]]] = {
    "extract_status_code": status_code_answer,
    "is_invalid_api_key_request": invalid_key_answer,
    "should_skip_metrics_for_invalid_key": skip_answer,
    "extract_deployment_failure_label_values": deployment_answer,
    "get_exception_class_name": class_name_answer,
    "bounded_requested_model_label": requested_model_answer,
    "extract_rate_limit_labels": rate_limit_answer,
    "extract_api_provider_from_request_data": request_provider_answer,
    "extract_api_provider_from_exception": exception_provider_answer,
}


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    return HANDLERS[function](input_value)
