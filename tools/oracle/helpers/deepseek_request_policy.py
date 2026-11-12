import logging
from types import TracebackType
from typing import Final, Literal, Self, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.deepseek.chat.transformation import DeepSeekChatConfig
from litellm.types.llms.openai import AllMessageValues
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
LOGGER_NAME: Final = "LiteLLM"
EMPTY: Final[dict[str, Json]] = {}
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


class RequestCase(TypedDict):
    model: str
    messages: list[Message]
    optional_params: dict[str, Json]
    supports_vision: bool
    supports_reasoning: bool


class MapInput(TypedDict):
    op: Literal["map"]
    cases: list[MapCase]


class RequestInput(TypedDict):
    op: Literal["request"]
    cases: list[RequestCase]


Input: TypeAlias = MapInput | RequestInput


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
    else:
        for request_case in input_value["cases"]:
            entry: dict[str, Json] = {
                "litellm_provider": "deepseek",
                "mode": "chat",
                "supports_vision": request_case["supports_vision"],
                "supports_reasoning": request_case["supports_reasoning"],
            }
            litellm.model_cost[f"deepseek/{request_case['model']}"] = entry  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
            with Capture() as capture:
                body = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L296
                    "dict[str, Json]",
                    DeepSeekChatConfig().transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L296
                        model=request_case["model"],
                        messages=cast("list[AllMessageValues]", request_case["messages"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
                        optional_params=request_case["optional_params"],
                        litellm_params=EMPTY,
                        headers=EMPTY,
                    ),
                )
            results.append({"value": body, "warnings": capture.events})
    return {input_value["op"]: results}
