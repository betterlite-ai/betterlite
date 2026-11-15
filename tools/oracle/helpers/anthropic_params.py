import logging
from collections.abc import Generator
from contextlib import contextmanager
from typing import Literal, Protocol, TypeAlias, TypedDict, TypeVar

import litellm
from litellm.exceptions import BadRequestError
from litellm.llms.anthropic.chat.transformation import AnthropicConfig
from pydantic import JsonValue as Json
from typing_extensions import override

Params = dict[str, Json]
MessageT = TypeVar("MessageT")
# the class object itself carries Any, so it enters through an annotated name
RaisedError: type[Exception] = BadRequestError


class Fixture(TypedDict):
    model_cost: dict[str, Json]
    packaged: dict[str, Json]


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


class Transformer(Protocol[MessageT]):
    def transform_request(
        self,
        model: str,
        messages: list[MessageT],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        headers: dict[str, Json],
    ) -> dict[str, Json]: ...


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())

    def first(self) -> str | None:
        return self.messages[0] if self.messages else None


@contextmanager
def capturing(*, global_drop_params: bool, modify_params: bool) -> Generator[Warnings]:
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    previous_drop = litellm.drop_params
    previous_modify = litellm.modify_params
    litellm.drop_params = global_drop_params
    litellm.modify_params = modify_params
    try:
        yield warnings
    finally:
        litellm.drop_params = previous_drop
        litellm.modify_params = previous_modify
        logger.removeHandler(warnings)


def map_answer(config: ParamMapper, step: MapStep) -> Json:
    optional_params: Params = dict(step["optional_params"])
    with capturing(global_drop_params=step["global_drop_params"], modify_params=False) as warnings:
        mapped = config.map_openai_params(
            non_default_params=step["non_default_params"],
            optional_params=optional_params,
            model=step["model"],
            drop_params=step["drop_params"],
        )
    return {"optional_params": mapped, "warning": warnings.first()}


def output_answer(request: Transformer[MessageT], step: OutputStep) -> Json:
    optional_params: Params = dict(step["optional_params"])
    litellm_params: Params = {}
    headers: Params = {}
    messages: list[MessageT] = []
    with capturing(global_drop_params=step["global_drop_params"], modify_params=True) as warnings:
        data = request.transform_request(
            model=step["model"],
            messages=messages,
            optional_params=optional_params,
            litellm_params=litellm_params,
            headers=headers,
        )
    return {
        "data": {
            "output_config": data.get("output_config"),
            "output_config_present": "output_config" in data,
        },
        "optional_params": {
            "output_config": optional_params.get("output_config"),
            "output_config_present": "output_config" in optional_params,
        },
        "warning": warnings.first(),
    }


def answer(step: Step) -> Json:
    try:
        if step["function"] == "map_openai_params":
            return map_answer(AnthropicConfig(), step)
        return output_answer(AnthropicConfig(), step)
    except RaisedError:
        return {"error": {"raised": True}}


def run(input_value: Input) -> list[Json]:
    litellm.model_cost = input_value["fixture"]["model_cost"]
    # the proxy and Router set this, else an unknown model prints a banner to stdout
    litellm.suppress_debug_info = True
    return [answer(step) for step in input_value["steps"]]
