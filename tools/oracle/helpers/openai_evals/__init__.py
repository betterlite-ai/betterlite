from collections.abc import Callable
from typing import Final

import litellm
from pydantic import JsonValue

from .probes import (
    CaseInput,
    Probe,
    create_update_probe,
    id_probe,
    request_probe,
)
from .replies import auth_probe, reply_probe, url_probe

litellm.suppress_debug_info = True

HANDLERS: Final[dict[str, Callable[[Probe], JsonValue]]] = {
    "eval_create_update": create_update_probe,
    "eval_list_pagination": request_probe,
    "eval_get_delete_cancel": id_probe,
    "run_create_data_source": request_probe,
    "run_list_pagination": request_probe,
    "run_get_cancel_delete": id_probe,
    "malformed_json_error": reply_probe,
    "eval_auth_required": auth_probe,
    "eval_url_literal_append": url_probe,
}


def run(input_value: dict[str, CaseInput]) -> JsonValue:
    if len(input_value) != 1:
        raise ValueError("expected one openai_evals case")
    name = next(iter(input_value))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown openai_evals case: {name}")
    return [handler(probe) for probe in input_value[name]["probes"]]
