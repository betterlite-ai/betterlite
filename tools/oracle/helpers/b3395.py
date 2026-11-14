import asyncio
import logging
from collections import deque
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.proxy.hooks.litellm_skills.main import SkillsInjectionHook
from litellm.types.llms.anthropic_messages.anthropic_response import AnthropicMessagesResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

PREFIX: Final = "SkillsInjectionHook: "
# betterlite's execute_code belongs to B3394 and does not log this line
EXECUTING: Final = "SkillsInjectionHook: Executing code ("
RESPONSE: Final = TypeAdapter(AnthropicMessagesResponse)
JSON: Final = TypeAdapter(JsonValue)

Object: TypeAlias = dict[str, JsonValue]
Entry: TypeAlias = None | bool | int | float | str | bytes | list["Entry"] | dict[str, "Entry"]


class ResponseReply(TypedDict):
    kind: Literal["response"]
    response: Object


class AbsentReply(TypedDict):
    kind: Literal["none"]


class ErrorReply(TypedDict):
    kind: Literal["error"]
    message: str


Reply: TypeAlias = ResponseReply | AbsentReply | ErrorReply


class Input(TypedDict):
    max_iterations: int
    enabled: bool
    model: str
    messages: list[Entry]
    tools: list[Entry]
    max_tokens: NotRequired[int]
    skill_files: dict[str, dict[str, str]]
    response: Object
    replies: list[Reply]


class Lines(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.lines: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if message.startswith(PREFIX) and not message.startswith(EXECUTING):
            self.lines.append({"level": record.levelname, "message": message})


class ScriptedMessages:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = deque(replies)
        self.calls: list[JsonValue] = []

    async def acreate(
        self,
        *,
        model: str,
        messages: list[Entry],
        tools: list[Entry],
        max_tokens: int,
    ) -> AnthropicMessagesResponse | None:
        self.calls.append(
            JSON.validate_python(
                {"model": model, "messages": messages, "tools": tools, "max_tokens": max_tokens}
            )
        )
        reply = self.replies.popleft()
        if reply["kind"] == "error":
            raise RuntimeError(reply["message"])
        if reply["kind"] == "none":
            return None
        return RESPONSE.validate_python(reply["response"])


def run(input_value: Input) -> Object:
    handler = Lines()
    logger = logging.getLogger("LiteLLM Proxy")
    logger.setLevel(logging.DEBUG)
    logger.addHandler(handler)
    scripted = ScriptedMessages(input_value["replies"])
    litellm.anthropic.acreate = scripted.acreate  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/litellm_skills/main.py#L590
    files: dict[str, Entry] = {
        skill_id: {path: text.encode() for path, text in paths.items()}
        for skill_id, paths in input_value["skill_files"].items()
    }
    metadata: dict[str, Entry] = {
        "_litellm_code_execution_enabled": input_value["enabled"],
        "_skill_files": files,
    }
    request_data: dict[str, Entry] = {
        "model": input_value["model"],
        "messages": input_value["messages"],
        "tools": input_value["tools"],
        "litellm_metadata": metadata,
    }
    if "max_tokens" in input_value:
        request_data["max_tokens"] = input_value["max_tokens"]
    hook = SkillsInjectionHook(max_iterations=input_value["max_iterations"])
    outcome = asyncio.run(
        hook.async_post_call_success_deployment_hook(
            request_data, RESPONSE.validate_python(input_value["response"]), None
        )
    )
    return {
        "result": JSON.validate_python(outcome),
        "continuation_calls": scripted.calls,
        "logs": handler.lines,
    }
