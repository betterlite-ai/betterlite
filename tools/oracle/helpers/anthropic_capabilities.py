import logging
from typing import Literal, TypeAlias, TypedDict, cast

import litellm
from litellm.exceptions import BadRequestError
from litellm.llms.anthropic.chat.transformation import AnthropicConfig
from litellm.types.llms.anthropic import AnthropicThinkingParam
from pydantic import JsonValue as Json
from typing_extensions import override

JsonObject: TypeAlias = dict[str, Json]


class Fixture(TypedDict):
    model_cost: JsonObject
    packaged: JsonObject


class MapStep(TypedDict):
    function: Literal["map_reasoning_effort"]
    model: str
    custom_llm_provider: str
    llm_provider: str
    reasoning_effort: str | None


class CapStep(TypedDict):
    function: Literal["cap_thinking_budget_to_max_tokens"]
    thinking: AnthropicThinkingParam
    max_tokens: int | None


class ValidateStep(TypedDict):
    function: Literal["validate_effort_for_model"]
    model: str
    custom_llm_provider: str
    effort: str | None


class EffortParamStep(TypedDict):
    function: Literal["model_supports_effort_param"]
    model: str
    custom_llm_provider: str


class InvalidStep(TypedDict):
    function: Literal["raise_invalid_reasoning_effort"]
    model: str
    llm_provider: str
    value: str


class SpeedSupportStep(TypedDict):
    function: Literal["model_supports_speed_param"]
    model: str
    custom_llm_provider: str | None


class SpeedDropStep(TypedDict):
    function: Literal["maybe_drop_speed_param"]
    model: str
    custom_llm_provider: str | None
    params: JsonObject
    drop_params: bool
    global_drop_params: bool


class SupportedParamsStep(TypedDict):
    function: Literal["get_supported_openai_params"]
    model: str


Step: TypeAlias = (
    MapStep
    | CapStep
    | ValidateStep
    | EffortParamStep
    | InvalidStep
    | SpeedSupportStep
    | SpeedDropStep
    | SupportedParamsStep
)


class Input(TypedDict):
    fixture: Fixture
    steps: list[Step]


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())

    def first(self) -> str | None:
        return self.messages[0] if self.messages else None


class Bundled:
    def __init__(self, model_cost: JsonObject) -> None:
        self.model_cost = model_cost

    def __call__(self) -> JsonObject:
        return self.model_cost


def map_reasoning_effort(step: MapStep) -> Json:
    mapped = AnthropicConfig._map_reasoning_effort(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L1251
        reasoning_effort=step["reasoning_effort"],
        model=step["model"],
        custom_llm_provider=step["custom_llm_provider"],
        llm_provider=step["llm_provider"],
    )
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L1251
        Json, mapped
    )


def cap_thinking_budget(step: CapStep) -> Json:
    fitted = AnthropicConfig.cap_thinking_budget_to_max_tokens(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L1312
        step["thinking"], step["max_tokens"]
    )
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L1312
        Json, fitted
    )


def validate_effort_for_model(step: ValidateStep) -> Json:
    return AnthropicConfig._validate_effort_for_model(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L406
        model=step["model"],
        effort=step["effort"],
        custom_llm_provider=step["custom_llm_provider"],
    )


def model_supports_effort_param(step: EffortParamStep) -> Json:
    return AnthropicConfig._model_supports_effort_param(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L418
        step["model"], step["custom_llm_provider"]
    )


def model_supports_speed_param(step: SpeedSupportStep) -> Json:
    return AnthropicConfig._model_supports_speed_param(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L434
        step["model"], step["custom_llm_provider"]
    )


def raise_invalid_reasoning_effort(step: InvalidStep) -> Json:
    AnthropicConfig._raise_invalid_reasoning_effort(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L474
        model=step["model"],
        value=step["value"],
        llm_provider=step["llm_provider"],
    )


def maybe_drop_speed_param(step: SpeedDropStep) -> Json:
    params: JsonObject = dict(step["params"])
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    previous = litellm.drop_params
    litellm.drop_params = step["global_drop_params"]
    try:
        AnthropicConfig._maybe_drop_speed_param(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L447
            model=step["model"],
            optional_params=params,
            drop_params=step["drop_params"],
            custom_llm_provider=step["custom_llm_provider"],
        )
    finally:
        litellm.drop_params = previous
        logger.removeHandler(warnings)
    return {"params": params, "warning": warnings.first()}


def get_supported_openai_params(step: SupportedParamsStep) -> Json:
    config = AnthropicConfig()
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L496
        Json, config.get_supported_openai_params(step["model"])
    )


def answer(step: Step) -> Json:
    result: Json
    try:
        if step["function"] == "map_reasoning_effort":
            result = map_reasoning_effort(step)
        elif step["function"] == "cap_thinking_budget_to_max_tokens":
            result = cap_thinking_budget(step)
        elif step["function"] == "validate_effort_for_model":
            result = validate_effort_for_model(step)
        elif step["function"] == "model_supports_effort_param":
            result = model_supports_effort_param(step)
        elif step["function"] == "raise_invalid_reasoning_effort":
            result = raise_invalid_reasoning_effort(step)
        elif step["function"] == "model_supports_speed_param":
            result = model_supports_speed_param(step)
        elif step["function"] == "maybe_drop_speed_param":
            result = maybe_drop_speed_param(step)
        else:
            result = get_supported_openai_params(step)
    except BadRequestError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L219
        result = {"error": {"status": error.status_code}}
    return result


def run(input_value: Input) -> list[Json]:
    fixture = input_value["fixture"]
    model_cost: JsonObject = fixture["model_cost"]
    bundled = Bundled(fixture["packaged"])
    previous_cost = litellm.model_cost  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    previous_bundled = litellm.utils._get_bundled_model_cost_map  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L8419
    litellm.model_cost = model_cost
    # no public entry injects the packaged cost map, so the fixture replaces the private reader
    litellm.utils._get_bundled_model_cost_map = bundled  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L8419
    try:
        return [answer(step) for step in input_value["steps"]]
    finally:
        litellm.model_cost = previous_cost
        litellm.utils._get_bundled_model_cost_map = previous_bundled  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L8419
