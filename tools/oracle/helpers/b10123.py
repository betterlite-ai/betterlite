import logging
from typing import TypedDict, cast

from litellm.responses.mcp.litellm_proxy_mcp_handler import LiteLLM_Proxy_MCP_Handler
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

_JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Input(TypedDict):
    original_messages: list[JsonValue]
    body: dict[str, JsonValue]
    tool_results: list[dict[str, JsonValue]]


class Records(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.lines: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.lines.append({"level": record.levelname, "message": record.getMessage()})


def run(input_value: Input) -> dict[str, JsonValue]:
    response = ModelResponse(**input_value["body"])  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2181
    messages = cast("list[object]", input_value["original_messages"])  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L976
    handler = Records()
    logger = logging.getLogger("LiteLLM")
    previous_handlers = list(logger.handlers)
    previous_level = logger.level
    for existing in previous_handlers:
        logger.removeHandler(existing)
    logger.addHandler(handler)
    logger.setLevel(logging.DEBUG)
    try:
        follow_up = LiteLLM_Proxy_MCP_Handler._create_follow_up_messages_for_chat(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L975
            original_messages=messages,
            response=response,
            tool_results=input_value["tool_results"],
        )
    finally:
        logger.removeHandler(handler)
        for existing in previous_handlers:
            logger.addHandler(existing)
        logger.setLevel(previous_level)
    return {"messages": _JSON.validate_python(follow_up), "records": handler.lines}
