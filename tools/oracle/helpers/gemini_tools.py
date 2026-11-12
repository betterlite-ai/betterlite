import logging
from typing import TypeAlias, TypedDict

from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import (
    VertexGeminiConfig,
)
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

MODEL = "gemini-2.5-pro"


class Call(TypedDict):
    tools: Json
    functions: Json
    web_search_options: Json
    tool_choice: Json
    service_tier: Json
    server_side_invocations: bool
    tool_config: Json


class Input(TypedDict):
    calls: list[Call]
    settings: dict[str, str]


class WarningCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def measure(call: Call) -> Json:
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
    capture = WarningCapture()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(capture)
    try:
        VertexGeminiConfig().map_openai_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1086-L1240
            non_default_params=non_default,
            optional_params=optional_params,
            model=MODEL,
            drop_params=False,
        )
    finally:
        logger.removeHandler(capture)
    output: dict[str, Json] = {
        "tools": optional_params.get("tools"),
        "tool_choice": optional_params.get("tool_choice"),
        "toolConfig": optional_params.get("toolConfig"),
        "service_tier": optional_params.get("service_tier"),
        "warnings": len(capture.messages),
    }
    return output


def run(input_value: Input) -> Json:
    results: list[Json] = [measure(call) for call in input_value["calls"]]
    output: dict[str, Json] = {"results": results}
    return output
