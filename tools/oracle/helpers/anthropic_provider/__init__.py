import litellm
from litellm.llms.anthropic import get_anthropic_config
from pydantic import JsonValue, TypeAdapter

from .bindings import environment
from .completion import complete
from .native import batch, count, files
from .types import Case, Input, Observation


def observe(supplied: Input) -> Observation:
    match supplied["action"]:
        case "messages" | "legacy":
            return complete(supplied)
        case "config_url":
            config = get_anthropic_config(supplied["id"])
            return {
                "result": config.__name__, "post_call": False,
                "post_matches": True, "request": None,
            }
        case "count":
            return count(supplied)
        case "batch_create" | "batch_retrieve":
            return batch(supplied)
        case "batch_results" | "upload" | "retrieve" | "list" | "delete" | "content":
            return files(supplied)


def run(input_value: Case) -> JsonValue:
    litellm.suppress_debug_info = True
    results: list[Observation] = []
    for supplied in input_value["inputs"]:
        with environment(supplied):
            results.append(observe(supplied))
    return TypeAdapter(JsonValue).validate_python(results)
