import logging
from logging.handlers import MemoryHandler
from typing import Protocol, TypedDict

from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import (
    VertexGeminiConfig,
)
from pydantic import JsonValue as Json


class ParamMapper(Protocol):
    def map_openai_params(
        self,
        *,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        drop_params: bool,
    ) -> dict[str, Json]: ...


class Call(TypedDict):
    tools: Json
    functions: Json
    web_search_options: Json
    tool_choice: Json
    service_tier: Json
    server_side_invocations: bool
    tool_config: Json


class Input(TypedDict):
    call: Call
    settings: dict[str, str]


def count_warnings(
    config: ParamMapper,
    non_default: dict[str, Json],
    optional_params: dict[str, Json],
) -> int:
    capture = MemoryHandler(capacity=1000, flushLevel=logging.CRITICAL)
    capture.setLevel(logging.WARNING)
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(capture)
    try:
        config.map_openai_params(
            non_default_params=non_default,
            optional_params=optional_params,
            model="gemini-2.5-pro",
            drop_params=False,
        )
    finally:
        logger.removeHandler(capture)
    return len(capture.buffer)


def run(input_value: Input) -> Json:
    call = input_value["call"]
    optional_params: dict[str, Json] = {}
    if call["tool_config"] is not None:
        optional_params["toolConfig"] = call["tool_config"]
    non_default: dict[str, Json] = {
        "tools": call["tools"],
        "functions": call["functions"],
        "tool_choice": call["tool_choice"],
        "web_search_options": call["web_search_options"],
        "service_tier": call["service_tier"],
        "include_server_side_tool_invocations": call["server_side_invocations"],
    }
    warnings = count_warnings(VertexGeminiConfig(), non_default, optional_params)
    return {
        "tools": optional_params.get("tools"),
        "tool_choice": optional_params.get("tool_choice"),
        "toolConfig": optional_params.get("toolConfig"),
        "service_tier": optional_params.get("service_tier"),
        "warnings": warnings,
    }
