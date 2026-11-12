import asyncio
import logging
from types import TracebackType
from typing import Final, Literal, NotRequired, Self, TypeAlias, TypedDict, cast

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


class SupportedCase(TypedDict):
    model: str


class MapCase(TypedDict):
    model: str
    params: NotRequired[dict[str, Json]]
    optional_params: NotRequired[dict[str, Json]]


class Message(TypedDict, total=False):
    role: str
    content: str | list[dict[str, Json]]
    reasoning_content: str | None
    provider_specific_fields: dict[str, Json]
    search_results: list[dict[str, Json]]


class HistoryCase(TypedDict):
    messages: list[Message]


class MessagesCase(TypedDict):
    model: str
    messages: list[Message]
    supports_vision: bool


class ToolsCase(TypedDict):
    optional_params: dict[str, Json]


class RequestCase(TypedDict):
    model: str
    messages: list[Message]
    optional_params: NotRequired[dict[str, Json]]
    supports_vision: bool
    supports_reasoning: bool
    mode: NotRequired[Literal["sync", "async"]]


class SupportedInput(TypedDict):
    op: Literal["supported"]
    cases: list[SupportedCase]


class MapInput(TypedDict):
    op: Literal["map"]
    cases: list[MapCase]


class HistoryInput(TypedDict):
    op: Literal["history"]
    cases: list[HistoryCase]


class MessagesInput(TypedDict):
    op: Literal["messages"]
    cases: list[MessagesCase]


class ToolsInput(TypedDict):
    op: Literal["tools"]
    cases: list[ToolsCase]


class RequestInput(TypedDict):
    op: Literal["request"]
    cases: list[RequestCase]


Input: TypeAlias = (
    SupportedInput | MapInput | HistoryInput | MessagesInput | ToolsInput | RequestInput
)


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.WARNING)
        self.events: list[Json] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        event = next((name for fragment, name in EVENTS if fragment in message), "unrecognized")
        self.events.append({"level": record.levelname.lower(), "event": event})

    def take(self) -> list[Json]:
        taken = self.events
        self.events = []
        return [entry for entry in taken if entry != "unrecognized"]


class Capture:
    def __init__(self) -> None:
        self.logger = logging.getLogger(LOGGER_NAME)
        self.handlers = self.logger.handlers
        self.level = self.logger.level
        self.warnings = Warnings()

    def __enter__(self) -> Self:
        self.logger.handlers = [self.warnings]
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


def dict_of(value: dict[str, Json] | None) -> dict[str, Json]:
    if value is None:
        return {}
    return dict(value)


def install_capabilities(
    model: str, *, supports_vision: bool, supports_reasoning: bool
) -> None:
    entry: dict[str, Json] = {
        "litellm_provider": "deepseek",
        "mode": "chat",
        "supports_vision": supports_vision,
        "supports_reasoning": supports_reasoning,
    }
    litellm.model_cost[f"deepseek/{model}"] = entry  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563


def supported_case(config: DeepSeekChatConfig, case: SupportedCase) -> Json:
    with Capture() as capture:
        value = cast("list[Json]", [*config.get_supported_openai_params(case["model"])])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L21
    return {"value": value, "warnings": capture.warnings.take()}


def map_case(config: DeepSeekChatConfig, case: MapCase) -> Json:
    params = dict_of(case.get("params"))
    options = dict_of(case.get("optional_params"))
    model = case["model"]
    with Capture() as capture:
        value = cast(Json, config.map_openai_params(params, options, model, drop_params=False))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L29
    return {"value": value, "warnings": capture.warnings.take()}


def history_case(config: DeepSeekChatConfig, case: HistoryCase) -> Json:
    with Capture() as capture:
        turns = case["messages"]
        value = cast(Json, config._fill_reasoning_content(cast("list[AllMessageValues]", turns)))  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L64
    return {"value": value, "warnings": capture.warnings.take()}


def messages_case(config: DeepSeekChatConfig, case: MessagesCase) -> Json:
    model = case["model"]
    install_capabilities(model, supports_vision=case["supports_vision"], supports_reasoning=False)
    with Capture() as capture:
        turns = case["messages"]
        value = cast(Json, config._transform_messages(cast("list[AllMessageValues]", turns), model))  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L117
    return {"value": value, "warnings": capture.warnings.take()}


def tools_case(config: DeepSeekChatConfig, case: ToolsCase) -> Json:
    with Capture() as capture:
        value = cast(Json, config._drop_unsupported_tools(case["optional_params"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L228
    return {"value": value, "warnings": capture.warnings.take()}


def request_case(config: DeepSeekChatConfig, case: RequestCase) -> Json:
    model = case["model"]
    install_capabilities(
        model,
        supports_vision=case["supports_vision"],
        supports_reasoning=case["supports_reasoning"],
    )
    optional = dict_of(case.get("optional_params"))
    turns = case["messages"]
    with Capture() as capture:
        if case.get("mode") == "async":
            body = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L324
                Json,
                asyncio.run(
                    config.async_transform_request(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L324
                        model=model,
                        messages=cast("list[AllMessageValues]", turns),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
                        optional_params=optional,
                        litellm_params=EMPTY,
                        headers=EMPTY,
                    )
                ),
            )
        else:
            body = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L296
                Json,
                config.transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L296
                    model=model,
                    messages=cast("list[AllMessageValues]", turns),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
                    optional_params=optional,
                    litellm_params=EMPTY,
                    headers=EMPTY,
                ),
            )
    return {"value": body, "warnings": capture.warnings.take()}


def run(input_value: Input) -> Json:
    config = DeepSeekChatConfig()
    if input_value["op"] == "supported":
        return {"supported": [supported_case(config, case) for case in input_value["cases"]]}
    if input_value["op"] == "map":
        return {"mapped": [map_case(config, case) for case in input_value["cases"]]}
    if input_value["op"] == "history":
        return {"history": [history_case(config, case) for case in input_value["cases"]]}
    if input_value["op"] == "messages":
        return {"messages": [messages_case(config, case) for case in input_value["cases"]]}
    if input_value["op"] == "tools":
        return {"tools": [tools_case(config, case) for case in input_value["cases"]]}
    return {"bodies": [request_case(config, case) for case in input_value["cases"]]}
