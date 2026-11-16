import os
from collections.abc import Callable
from typing import Final

from litellm.llms.bedrock.responses.transformation import BedrockOpenAIResponsesConfig
from pydantic import JsonValue

from . import handlers as ops
from .models import Calls

HANDLERS: Final[dict[str, Callable[[Calls, JsonValue], JsonValue]]] = {
    "bedrock_responses_model_route": ops.route_cases,
    "responses_bearer_auth": ops.auth_cases,
    "responses_sigv4": ops.sigv4_cases,
    "response_option_gates": ops.option_cases,
    "native_file_search_flag": ops.option_cases,
    "inline_image_traversal": ops.traversal_cases,
    "remote_url_inlined": ops.fetch_cases,
    "responses_runtime_url": ops.url_cases,
    "inline_image_replacement": ops.replacement_cases,
    "codex_input_rewrite": ops.codex_cases,
}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    if len(input_value) != 1:
        raise ValueError("bedrock_services_responses takes exactly one operation")
    operation, payload = next(iter(input_value.items()))
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    # LiteLLM reads LITELLM_RUST per call; false pins the Python fallback to compare against
    os.environ["LITELLM_RUST"] = "false"
    return handler(BedrockOpenAIResponsesConfig(), payload)
