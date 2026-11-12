import logging
from collections.abc import MutableMapping
from typing import Literal, TypeAlias, TypedDict, cast

import litellm
import litellm.utils
from litellm.exceptions import UnsupportedParamsError
from litellm.llms.anthropic.common_utils import AnthropicModelInfo
from litellm.types.llms.anthropic import AnthropicMessagesToolChoice
from pydantic import JsonValue as Json
from typing_extensions import override

JsonObject: TypeAlias = dict[str, Json]


class Fixture(TypedDict):
    model_cost: JsonObject
    packaged: JsonObject


class CatalogStep(TypedDict):
    function: Literal["provider_qualified_catalog"]
    model: str
    custom_llm_provider: str


class ThinkingStep(TypedDict):
    function: Literal["adaptive_thinking", "disabled_thinking_warning", "effort_beta_gate"]
    model: str
    custom_llm_provider: str
    params: JsonObject


class ForcedToolStep(TypedDict):
    function: Literal["forced_tool_downgrade"]
    model: str
    tool_choice: AnthropicMessagesToolChoice
    drop_params: bool
    global_drop_params: bool


class SamplingStep(TypedDict):
    function: Literal["sampling_capability"]
    model: str
    param: Literal["temperature", "top_p", "top_k"]
    value: Json
    output_key: str
    drop_params: bool
    global_drop_params: bool


class LegacyBudgetStep(TypedDict):
    function: Literal["legacy_budget_effort"]
    model: str
    custom_llm_provider: str
    budget_tokens: int


Step: TypeAlias = CatalogStep | ThinkingStep | ForcedToolStep | SamplingStep | LegacyBudgetStep


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


def provider_qualified_catalog(step: CatalogStep) -> Json:
    model = step["model"]
    provider = step["custom_llm_provider"]
    return {
        "supports_adaptive_thinking": AnthropicModelInfo._is_adaptive_thinking_model(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L696
            model,
            provider,
        ),
        "thinking_always_on": AnthropicModelInfo._is_always_on_thinking_model(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L708
            model,
            provider,
        ),
        "supports_legacy_thinking": AnthropicModelInfo._supports_legacy_thinking(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L717
            model,
            provider,
        ),
    }


def adaptive_thinking(step: ThinkingStep) -> Json:
    params = dict(step["params"])
    mutable = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L747
        "MutableMapping[str, object]", params
    )
    AnthropicModelInfo.translate_legacy_thinking_for_adaptive_model(
        step["model"],
        mutable,
        step["custom_llm_provider"],
    )
    return params


def forced_tool_downgrade(step: ForcedToolStep) -> Json:
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    previous = litellm.drop_params
    litellm.drop_params = step["global_drop_params"]
    try:
        tool_choice = AnthropicModelInfo._apply_forced_tool_choice(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L555
            step["model"],
            step["tool_choice"],
            step["drop_params"],
        )
    finally:
        litellm.drop_params = previous
        logger.removeHandler(warnings)
    return {
        # the TypedDict the function returns is a JSON object at runtime
        "tool_choice": cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L555
            Json, tool_choice
        ),
        "warning": warnings.first(),
    }


def sampling_capability(step: SamplingStep) -> Json:
    optional_params: JsonObject = {}
    previous = litellm.drop_params
    litellm.drop_params = step["global_drop_params"]
    try:
        AnthropicModelInfo._apply_sampling_param(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L506
            optional_params,
            step["model"],
            step["param"],
            step["value"],
            step["drop_params"],
            step["output_key"],
        )
    finally:
        litellm.drop_params = previous
    return optional_params


def disabled_thinking_warning(step: ThinkingStep) -> Json:
    params = dict(step["params"])
    mutable = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L727
        "MutableMapping[str, object]", params
    )
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    try:
        AnthropicModelInfo.maybe_drop_disabled_thinking(
            step["model"],
            mutable,
            step["custom_llm_provider"],
        )
    finally:
        logger.removeHandler(warnings)
    return {"params": params, "warning": warnings.first()}


def answer(step: Step) -> Json:
    result: Json
    try:
        if step["function"] == "provider_qualified_catalog":
            result = provider_qualified_catalog(step)
        elif step["function"] == "adaptive_thinking":
            result = adaptive_thinking(step)
        elif step["function"] == "forced_tool_downgrade":
            result = forced_tool_downgrade(step)
        elif step["function"] == "sampling_capability":
            result = sampling_capability(step)
        elif step["function"] == "legacy_budget_effort":
            result = {"effort": AnthropicModelInfo._legacy_budget_to_effort(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L784
                model=step["model"],
                budget_tokens=step["budget_tokens"],
                custom_llm_provider=step["custom_llm_provider"],
            )}
        elif step["function"] == "disabled_thinking_warning":
            result = disabled_thinking_warning(step)
        else:
            result = {"used": AnthropicModelInfo().is_effort_used(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L795
                step["params"],
                step["model"],
                custom_llm_provider=step["custom_llm_provider"],
            )}
    except UnsupportedParamsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        result = {"error": {"status": error.status_code}}
    except (KeyError, TypeError, ValueError):
        result = {"error": {"status": None}}
    return result


def run(input_value: Input) -> list[Json]:
    model_cost: JsonObject = input_value["fixture"]["model_cost"]
    bundled = Bundled(input_value["fixture"]["packaged"])
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
