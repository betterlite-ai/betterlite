from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.exceptions import UnsupportedParamsError
from litellm.llms.vertex_ai.gemini import vertex_and_google_ai_studio_gemini as gemini_config
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import VertexGeminiConfig
from litellm.types.llms.anthropic import AnthropicThinkingParam
from pydantic import JsonValue as Json

JsonObject: TypeAlias = dict[str, Json]


class Settings(TypedDict):
    default_thinking_level_low: bool


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
    ReasoningEffortStep | ThinkingStep | FormatStep | ModalitiesStep | AudioStep | ServerFlagsStep
)


class Input(TypedDict):
    settings: Settings
    catalog: dict[str, bool]
    max_recurse_depth: NotRequired[int]
    steps: list[Step]


class Catalog:
    def __init__(self, entries: dict[str, bool]) -> None:
        self.entries = entries

    def __call__(self, model: str, custom_llm_provider: str | None, key: str) -> bool:
        if custom_llm_provider is not None or key != "supports_minimal_reasoning_effort":
            raise ValueError(f"the fixture catalog only answers {key}")
        return self.entries.get(model) is False


def reasoning_effort(step: ReasoningEffortStep) -> Json:
    try:
        VertexGeminiConfig._validate_thinking_config_conflicts(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L918
            optional_params=step["optional_params"],
            param_name="reasoning_effort",
        )
        if step["model"] is not None and VertexGeminiConfig._is_gemini_3_or_newer(step["model"]):  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L285
            return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L880
                Json,
                VertexGeminiConfig._map_reasoning_effort_to_thinking_level(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L880
                    step["effort"], step["model"]
                ),
            )
        return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L823
            Json,
            VertexGeminiConfig._map_reasoning_effort_to_thinking_budget(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L823
                step["effort"], step["model"]
            ),
        )
    except UnsupportedParamsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        return {"error": {"status": error.status_code}}


def thinking(step: ThinkingStep) -> Json:
    try:
        VertexGeminiConfig._validate_thinking_config_conflicts(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L918
            optional_params=step["optional_params"],
            param_name="thinking",
        )
    except UnsupportedParamsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        return {"error": {"status": error.status_code}}
    config = VertexGeminiConfig._map_thinking_param(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L958
        step["thinking"], model=step["model"]
    )
    return cast(Json, config)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L958


def format_response(step: FormatStep) -> Json:
    optional_params: JsonObject = dict(step["optional_params"])
    try:
        VertexGeminiConfig().apply_response_schema_transformation(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L778
            value=step["response_format"],
            optional_params=optional_params,
            model=step["model"],
        )
    except ValueError:
        return {"error": {"status": 500}}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L867
    return cast(Json, optional_params)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L778


def audio(step: AudioStep) -> Json:
    try:
        config = VertexGeminiConfig()._map_audio_params(step["audio"])  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1019
    except ValueError:
        return {"error": {"status": 500}}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L867
    return cast(Json, config)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1019


def server_flags(step: ServerFlagsStep) -> Json:
    optional_params: JsonObject = dict(step["optional_params"])
    VertexGeminiConfig._apply_include_server_side_tool_invocations(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1067
        step["non_default_params"], optional_params
    )
    return cast(Json, optional_params)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1067


def answer(step: Step) -> Json:
    if step["op"] == "reasoning_effort":
        return reasoning_effort(step)
    if step["op"] == "thinking":
        return thinking(step)
    if step["op"] == "format":
        return format_response(step)
    if step["op"] == "modalities":
        return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L991
            Json,
            VertexGeminiConfig().map_response_modalities(step["modalities"]),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L991
        )
    if step["op"] == "audio":
        return audio(step)
    return server_flags(step)


def run(input_value: Input) -> Json:
    litellm.enable_gemini_default_thinking_level_low = input_value["settings"]["default_thinking_level_low"]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L390
    gemini_config.is_explicitly_disabled_factory = Catalog(input_value["catalog"])  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L84
    return {"results": [answer(step) for step in input_value["steps"]]}
