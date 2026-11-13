from typing import Final, Literal, Protocol, TypeAlias, TypedDict, cast

import litellm
from litellm.constants import (
    DEFAULT_MAX_RECURSE_DEPTH as MAX_RECURSE_DEPTH,
)
from litellm.constants import (
    DEFAULT_REASONING_EFFORT_DISABLE_THINKING_BUDGET as DISABLE_BUDGET,
)
from litellm.constants import (
    DEFAULT_REASONING_EFFORT_HIGH_THINKING_BUDGET as HIGH_BUDGET,
)
from litellm.constants import (
    DEFAULT_REASONING_EFFORT_LOW_THINKING_BUDGET as LOW_BUDGET,
)
from litellm.constants import (
    DEFAULT_REASONING_EFFORT_MEDIUM_THINKING_BUDGET as MEDIUM_BUDGET,
)
from litellm.constants import (
    DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET as MINIMAL_BUDGET,
)
from litellm.constants import (
    DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH as FLASH_BUDGET,
)
from litellm.constants import (
    DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH_LITE as FLASH_LITE_BUDGET,
)
from litellm.constants import (
    DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_PRO as PRO_BUDGET,
)
from litellm.exceptions import UnsupportedParamsError
from litellm.llms.vertex_ai import common_utils
from litellm.llms.vertex_ai.gemini import vertex_and_google_ai_studio_gemini as gemini_config
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import VertexGeminiConfig
from litellm.types.llms.anthropic import AnthropicThinkingParam
from litellm.utils import is_explicitly_disabled_factory
from pydantic import JsonValue as Json

JsonObject: TypeAlias = dict[str, Json]

DEFAULT_THINKING_LEVEL_LOW: Final = litellm.enable_gemini_default_thinking_level_low


class Budgets(TypedDict):
    minimal: int
    minimal_flash: int
    minimal_pro: int
    minimal_flash_lite: int
    low: int
    medium: int
    high: int
    disable: int


class Settings(TypedDict):
    budgets: Budgets
    default_thinking_level_low: bool
    max_recurse_depth: int


class ReasoningEffortStep(TypedDict):
    op: Literal["reasoning_effort"]
    effort: str
    model: str | None
    optional_params: JsonObject


class ThinkingStep(TypedDict):
    op: Literal["thinking"]
    thinking: AnthropicThinkingParam
    model: str | None
    optional_params: JsonObject


class FormatStep(TypedDict):
    op: Literal["format"]
    response_format: JsonObject
    model: str
    optional_params: JsonObject


class ModalitiesStep(TypedDict):
    op: Literal["modalities"]
    modalities: list[str]


class AudioStep(TypedDict):
    op: Literal["audio"]
    audio: JsonObject


class ServerFlagsStep(TypedDict):
    op: Literal["server_flags"]
    non_default_params: JsonObject
    optional_params: JsonObject


Step: TypeAlias = (
    ReasoningEffortStep
    | ThinkingStep
    | FormatStep
    | ModalitiesStep
    | AudioStep
    | ServerFlagsStep
)


class Input(TypedDict):
    settings: Settings
    catalog: dict[str, bool]
    steps: list[Step]


class Config(Protocol):
    # ast-grep-ignore: no-long-names
    def _validate_thinking_config_conflicts(
        self, optional_params: dict[str, Json], param_name: str
    ) -> None: ...

    def _is_gemini_3_or_newer(self, model: str) -> bool: ...

    # ast-grep-ignore: no-long-names
    def _map_reasoning_effort_to_thinking_budget(
        self, reasoning_effort: str, model: str | None
    ) -> Json: ...

    # ast-grep-ignore: no-long-names
    def _map_reasoning_effort_to_thinking_level(
        self, reasoning_effort: str, model: str | None
    ) -> Json: ...

    def _map_thinking_param(
        self, thinking_param: AnthropicThinkingParam, model: str | None
    ) -> Json: ...

    # ast-grep-ignore: no-long-names
    def apply_response_schema_transformation(
        self, value: JsonObject, optional_params: JsonObject, model: str
    ) -> None: ...

    def map_response_modalities(self, value: list[str]) -> list[str]: ...

    def _map_audio_params(self, value: JsonObject) -> Json: ...

    # ast-grep-ignore: no-long-names
    def _apply_include_server_side_tool_invocations(
        self, non_default_params: JsonObject, optional_params: JsonObject
    ) -> None: ...


CONFIG: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L184
    Config, VertexGeminiConfig()
)


class Catalog:
    def __init__(self, entries: dict[str, bool]) -> None:
        self.entries = entries

    def __call__(self, model: str, custom_llm_provider: str | None, key: str) -> bool:
        if custom_llm_provider is not None or key != "supports_minimal_reasoning_effort":
            raise ValueError(f"the fixture catalog only answers {key}")
        return self.entries.get(model) is False


DEFAULT_BUDGETS: Final[Budgets] = {
    "minimal": MINIMAL_BUDGET,
    "minimal_flash": FLASH_BUDGET,
    "minimal_pro": PRO_BUDGET,
    "minimal_flash_lite": FLASH_LITE_BUDGET,
    "low": LOW_BUDGET,
    "medium": MEDIUM_BUDGET,
    "high": HIGH_BUDGET,
    "disable": DISABLE_BUDGET,
}


def apply_budgets(budgets: Budgets, *, default_thinking_level_low: bool) -> None:
    gemini_config.DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET = budgets["minimal"]  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L263
    gemini_config.DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH = budgets["minimal_flash"]  # type: ignore[attr-defined]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L234
    gemini_config.DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_PRO = budgets["minimal_pro"]  # type: ignore[attr-defined]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L237
    gemini_config.DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH_LITE = budgets["minimal_flash_lite"]  # type: ignore[attr-defined]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L240
    gemini_config.DEFAULT_REASONING_EFFORT_LOW_THINKING_BUDGET = budgets["low"]  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L283
    gemini_config.DEFAULT_REASONING_EFFORT_MEDIUM_THINKING_BUDGET = budgets["medium"]  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L284
    gemini_config.DEFAULT_REASONING_EFFORT_HIGH_THINKING_BUDGET = budgets["high"]  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L287
    gemini_config.DEFAULT_REASONING_EFFORT_DISABLE_THINKING_BUDGET = budgets["disable"]  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L154
    litellm.enable_gemini_default_thinking_level_low = default_thinking_level_low


def install(settings: Settings) -> None:
    apply_budgets(
        settings["budgets"],
        default_thinking_level_low=settings["default_thinking_level_low"],
    )
    common_utils.DEFAULT_MAX_RECURSE_DEPTH = settings["max_recurse_depth"]  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L15


def restore() -> None:
    apply_budgets(DEFAULT_BUDGETS, default_thinking_level_low=DEFAULT_THINKING_LEVEL_LOW)
    common_utils.DEFAULT_MAX_RECURSE_DEPTH = MAX_RECURSE_DEPTH  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L15
    gemini_config.is_explicitly_disabled_factory = is_explicitly_disabled_factory  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L84


def reasoning_effort(step: ReasoningEffortStep) -> Json:
    try:
        CONFIG._validate_thinking_config_conflicts(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L918
            optional_params=step["optional_params"],
            param_name="reasoning_effort",
        )
        if step["model"] is not None and CONFIG._is_gemini_3_or_newer(step["model"]):  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L285
            return CONFIG._map_reasoning_effort_to_thinking_level(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L880
                step["effort"], step["model"]
            )
        return CONFIG._map_reasoning_effort_to_thinking_budget(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L823
            step["effort"], step["model"]
        )
    except UnsupportedParamsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        return {"error": {"status": error.status_code}}


def thinking(step: ThinkingStep) -> Json:
    try:
        CONFIG._validate_thinking_config_conflicts(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L918
            optional_params=step["optional_params"],
            param_name="thinking",
        )
    except UnsupportedParamsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        return {"error": {"status": error.status_code}}
    return CONFIG._map_thinking_param(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L958
        step["thinking"], model=step["model"]
    )


def format_response(step: FormatStep) -> Json:
    optional_params: JsonObject = dict(step["optional_params"])
    try:
        CONFIG.apply_response_schema_transformation(
            value=step["response_format"],
            optional_params=optional_params,
            model=step["model"],
        )
    except ValueError:
        return {"error": {"status": None}}
    return cast(Json, optional_params)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L778


def modalities(step: ModalitiesStep) -> Json:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L991
        Json, CONFIG.map_response_modalities(step["modalities"])
    )


def audio(step: AudioStep) -> Json:
    try:
        return CONFIG._map_audio_params(step["audio"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1019
    except ValueError:
        return {"error": {"status": None}}


def server_flags(step: ServerFlagsStep) -> Json:
    optional_params: JsonObject = dict(step["optional_params"])
    CONFIG._apply_include_server_side_tool_invocations(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1067
        step["non_default_params"], optional_params
    )
    return cast(Json, optional_params)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1067


def answer(step: Step) -> Json:
    result: Json
    if step["op"] == "reasoning_effort":
        result = reasoning_effort(step)
    elif step["op"] == "thinking":
        result = thinking(step)
    elif step["op"] == "format":
        result = format_response(step)
    elif step["op"] == "modalities":
        result = modalities(step)
    elif step["op"] == "audio":
        result = audio(step)
    else:
        result = server_flags(step)
    return result


def run(input_value: Input) -> Json:
    settings = input_value["settings"]
    install(settings)
    gemini_config.is_explicitly_disabled_factory = Catalog(input_value["catalog"])  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L84
    try:
        return {"results": [answer(step) for step in input_value["steps"]]}
    finally:
        restore()
