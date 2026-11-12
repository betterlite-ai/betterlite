import logging
from types import TracebackType
from typing import Final, Literal, Self, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.deepseek.chat.transformation import DeepSeekChatConfig
from litellm.types.llms.openai import AllMessageValues
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
LOGGER_NAME: Final = "LiteLLM"
REASONER: Final = "deepseek-reasoner"
THINKING_ENABLED: Final[dict[str, Json]] = {"type": "enabled"}
EMPTY: Final[dict[str, Json]] = {}
BODY_KEYS: Final = ("tools", "tool_choice", "parallel_tool_calls", "temperature")
EVENTS: Final[tuple[tuple[str, str], ...]] = (
    ("only supports function tools", "tools_dropped"),
    ("assistant message is missing", "reasoning_placeholder"),
)


class Message(TypedDict, total=False):
    role: str
    content: str | list[dict[str, Json]]
    reasoning_content: str | None
    provider_specific_fields: dict[str, Json]
    search_results: list[dict[str, Json]]


class MapCase(TypedDict):
    model: str
    params: dict[str, Json]
    optional_params: dict[str, Json]


class HistoryCase(TypedDict):
    messages: list[Message]


class ToolsCase(TypedDict):
    optional_params: dict[str, Json]


class MessagesCase(TypedDict):
    model: str
    messages: list[Message]
    supports_vision: bool


class MapInput(TypedDict):
    op: Literal["map"]
    cases: list[MapCase]


class HistoryInput(TypedDict):
    op: Literal["history"]
    cases: list[HistoryCase]


class ToolsInput(TypedDict):
    op: Literal["tools"]
    cases: list[ToolsCase]


class MessagesInput(TypedDict):
    op: Literal["messages"]
    cases: list[MessagesCase]


Input: TypeAlias = MapInput | HistoryInput | ToolsInput | MessagesInput


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.WARNING)
        self.logger = logging.getLogger(LOGGER_NAME)
        self.handlers = self.logger.handlers
        self.level = self.logger.level
        self.events: list[Json] = []

    def __enter__(self) -> Self:
        self.logger.handlers = [self]
        self.logger.setLevel(logging.DEBUG)
        return self

    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.logger.handlers = self.handlers
        self.logger.setLevel(self.level)

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        event = next((name for fragment, name in EVENTS if fragment in message), None)
        if event is not None:
            self.events.append({"level": record.levelname.lower(), "event": event})


def request_body(
    model: str,
    messages: list[Message],
    optional_params: dict[str, Json],
    *,
    supports_vision: bool,
    supports_reasoning: bool,
) -> dict[str, Json]:
    entry: dict[str, Json] = {
        "litellm_provider": "deepseek",
        "mode": "chat",
        "supports_vision": supports_vision,
        "supports_reasoning": supports_reasoning,
    }
    litellm.model_cost[f"deepseek/{model}"] = entry  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L296
        "dict[str, Json]",
        DeepSeekChatConfig().transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L296
            model=model,
            messages=cast("list[AllMessageValues]", messages),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            optional_params=optional_params,
            litellm_params=EMPTY,
            headers=EMPTY,
        ),
    )


def run(input_value: Input) -> Json:
    results: list[Json] = []
    if input_value["op"] == "map":
        for map_case in input_value["cases"]:
            with Capture() as capture:
                mapped = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L29
                    "dict[str, Json]",
                    DeepSeekChatConfig().map_openai_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L29
                        map_case["params"],
                        map_case["optional_params"],
                        map_case["model"],
                        drop_params=False,
                    ),
                )
            results.append({"value": mapped, "warnings": capture.events})
    elif input_value["op"] == "history":
        for history_case in input_value["cases"]:
            with Capture() as capture:
                body = request_body(
                    REASONER,
                    history_case["messages"],
                    {"thinking": THINKING_ENABLED},
                    supports_vision=False,
                    supports_reasoning=True,
                )
            results.append({"value": body["messages"], "warnings": capture.events})
    elif input_value["op"] == "tools":
        for tools_case in input_value["cases"]:
            with Capture() as capture:
                body = request_body(
                    REASONER,
                    [],
                    {**tools_case["optional_params"], "thinking": THINKING_ENABLED},
                    supports_vision=False,
                    supports_reasoning=True,
                )
            kept: dict[str, Json] = {key: body[key] for key in BODY_KEYS if key in body}
            results.append({"value": kept, "warnings": capture.events})
    else:
        for messages_case in input_value["cases"]:
            with Capture() as capture:
                body = request_body(
                    messages_case["model"],
                    messages_case["messages"],
                    {},
                    supports_vision=messages_case["supports_vision"],
                    supports_reasoning=False,
                )
            results.append({"value": body["messages"], "warnings": capture.events})
    return {input_value["op"]: results}
