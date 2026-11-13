import logging
from collections.abc import Callable
from typing import Literal, Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.anthropic.chat.transformation import AnthropicConfig
from pydantic import JsonValue as Json
from typing_extensions import override


class Budgets(TypedDict):
    minimal: int
    low: int
    medium: int
    high: int
    xhigh: int
    max: int


class Fixture(TypedDict):
    model_cost: dict[str, Json]
    packaged: dict[str, Json]
    budgets: Budgets
    default_max_tokens: int


class MapStep(TypedDict):
    function: Literal["map_openai_params"]
    model: str
    non_default_params: dict[str, Json]
    optional_params: dict[str, Json]
    drop_params: bool
    global_drop_params: bool


class OutputStep(TypedDict):
    function: Literal["apply_output_config"]
    model: str
    data: dict[str, Json]
    optional_params: dict[str, Json]
    global_drop_params: bool


Step: TypeAlias = MapStep | OutputStep


class Input(TypedDict):
    fixture: Fixture
    steps: list[Step]


class ParamMapper(Protocol):
    def map_openai_params(
        self,
        *,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        drop_params: bool,
    ) -> dict[str, Json]: ...


class Transformer(Protocol):
    transform_request: Callable[..., dict[str, Json]]


class CostMapHost(Protocol):
    model_cost: dict[str, Json]


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())

    def first(self) -> str | None:
        return self.messages[0] if self.messages else None


def map_answer(config: ParamMapper, step: MapStep) -> Json:
    optional_params: dict[str, Json] = dict(step["optional_params"])
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    previous = litellm.drop_params
    litellm.drop_params = step["global_drop_params"]
    try:
        mapped = config.map_openai_params(
            non_default_params=step["non_default_params"],
            optional_params=optional_params,
            model=step["model"],
            drop_params=step["drop_params"],
        )
    finally:
        litellm.drop_params = previous
        logger.removeHandler(warnings)
    return {"optional_params": mapped, "warning": warnings.first()}


def output_answer(request: Transformer, step: OutputStep) -> Json:
    optional_params: dict[str, Json] = dict(step["optional_params"])
    messages: list[dict[str, Json]] = [{"role": "user", "content": "hello"}]
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    previous = litellm.drop_params
    litellm.drop_params = step["global_drop_params"]
    try:
        data: dict[str, Json] = request.transform_request(
            model=step["model"],
            messages=messages,
            optional_params=optional_params,
            litellm_params={},
            headers={},
        )
    finally:
        litellm.drop_params = previous
        logger.removeHandler(warnings)
    return {
        "data": {"output_config": data.get("output_config")},
        "optional_params": {"output_config": optional_params.get("output_config")},
        "warning": warnings.first(),
    }


def answer(step: Step) -> Json:
    if step["function"] == "map_openai_params":
        return map_answer(AnthropicConfig(), step)
    return output_answer(AnthropicConfig(), step)


def run(input_value: Input) -> list[Json]:
    host: CostMapHost = litellm
    host.model_cost = input_value["fixture"]["model_cost"]
    return [answer(step) for step in input_value["steps"]]
