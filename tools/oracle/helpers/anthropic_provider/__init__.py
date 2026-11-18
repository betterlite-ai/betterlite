import os
from unittest import mock

import litellm
from litellm.llms.anthropic import get_anthropic_config
from pydantic import JsonValue, TypeAdapter

from .completion import complete, selection_failure
from .native import batch, count, files
from .types import Case, Input, Observation


def observe(supplied: Input) -> Observation:
    match supplied["action"]:
        case "messages" | "legacy":
            if not supplied["config_present"] or supplied["provider"] == "not-a-provider":
                return selection_failure(supplied)
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
        environment = dict(supplied["environment"])
        environment["LITELLM_RUST"] = "false"
        environment["LITELLM_LOCAL_MODEL_COST_MAP"] = "true"
        environment["LITELLM_ANTHROPIC_DISABLE_URL_SUFFIX"] = str(supplied["disable_suffix"]).lower()
        with (
            mock.patch.dict(os.environ, environment, clear=True),
            mock.patch.object(litellm, "api_base", supplied["global_base"]),
            mock.patch.object(litellm, "api_key", supplied["global_key"]),
            mock.patch.object(litellm, "anthropic_key", supplied["anthropic_key"]),
            mock.patch.object(litellm, "custom_prompt_dict", supplied["global_prompt_dict"]),
        ):
            results.append(observe(supplied))
    return TypeAdapter(JsonValue).validate_python(results)
