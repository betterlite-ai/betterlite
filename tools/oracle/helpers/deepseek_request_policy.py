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


class Case(TypedDict, total=False):
    model: str
    params: dict[str, Json]
    optional_params: dict[str, Json]
    messages: list[Message]
    supports_vision: bool


class Input(TypedDict):
    op: Literal["map", "history", "tools", "messages"]
    cases: list[Case]


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
        event = next((name for fragment, name in EVENTS if fragment in message), "unrecognized")
        self.events.append({"level": record.levelname.lower(), "event": event})

    def take(self) -> list[Json]:
        taken = self.events
        self.events = []
        return [entry for entry in taken if entry != "unrecognized"]


def install_capabilities(model: str, *, supports_vision: bool) -> None:
    entry: dict[str, Json] = {
        "litellm_provider": "deepseek",
        "mode": "chat",
        "supports_vision": supports_vision,
        "supports_reasoning": False,
    }
    litellm.model_cost[f"deepseek/{model}"] = entry  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563


def render(config: DeepSeekChatConfig, op: str, case: Case) -> Json:
    model = case["model"]  # pyright: ignore[reportTypedDictNotRequiredAccess]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L296
    if op == "map":
        params = case["params"]  # pyright: ignore[reportTypedDictNotRequiredAccess]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L29
        options = case["optional_params"]  # pyright: ignore[reportTypedDictNotRequiredAccess]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L29
        return cast(Json, config.map_openai_params(params, options, model, drop_params=False))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L29
    if op == "tools":
        return cast(Json, config._drop_unsupported_tools(case["optional_params"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportTypedDictNotRequiredAccess]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L228
    turns = case["messages"]  # pyright: ignore[reportTypedDictNotRequiredAccess]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L117
    if op == "history":
        return cast(Json, config._fill_reasoning_content(cast("list[AllMessageValues]", turns)))  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L64
    install_capabilities(model, supports_vision=case["supports_vision"])  # pyright: ignore[reportTypedDictNotRequiredAccess]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L129
    body = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L296
        "dict[str, Json]",
        config.transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/chat/transformation.py#L296
            model=model,
            messages=cast("list[AllMessageValues]", turns),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            optional_params=EMPTY,
            litellm_params=EMPTY,
            headers=EMPTY,
        ),
    )
    return body["messages"]


def run(input_value: Input) -> Json:
    config = DeepSeekChatConfig()
    results: list[Json] = []
    for case in input_value["cases"]:
        with Capture() as capture:
            value = render(config, input_value["op"], case)
        results.append({"value": value, "warnings": capture.take()})
    return {input_value["op"]: results}
