from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import VertexGeminiConfig
from openai import BadRequestError
from pydantic import JsonValue as Json

JsonObject: TypeAlias = dict[str, Json]

UNRELATED_MODEL = "gemini-2.5-flash"


class ParamMapper(Protocol):
    def map_openai_params(
        self,
        *,
        non_default_params: JsonObject,
        optional_params: JsonObject,
        model: str,
        drop_params: bool,
    ) -> JsonObject: ...


TYPED_CONFIG: ParamMapper = VertexGeminiConfig()


class Settings(TypedDict):
    default_thinking_level_low: bool


class ReasoningEffortStep(TypedDict):
    op: Literal["reasoning_effort"]
    effort: str
    model: str
    optional_params: JsonObject


class ThinkingStep(TypedDict):
    op: Literal["thinking"]
    thinking: JsonObject
    model: str
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


def answer(step: Step) -> Json:
    non_default_params: JsonObject = {}
    optional_params: JsonObject = {}
    model = UNRELATED_MODEL
    projection: str | None = None
    if step["op"] == "reasoning_effort":
        non_default_params = {"reasoning_effort": step["effort"]}
        optional_params = dict(step["optional_params"])
        model = step["model"]
        projection = "thinkingConfig"
    elif step["op"] == "thinking":
        non_default_params = {"thinking": step["thinking"]}
        optional_params = dict(step["optional_params"])
        model = step["model"]
        projection = "thinkingConfig"
    elif step["op"] == "format":
        non_default_params = {"response_format": step["response_format"]}
        optional_params = dict(step["optional_params"])
        model = step["model"]
    elif step["op"] == "modalities":
        non_default_params = {"modalities": list(step["modalities"])}
        projection = "responseModalities"
    elif step["op"] == "audio":
        non_default_params = {"audio": step["audio"]}
        projection = "speechConfig"
    else:
        non_default_params = step["non_default_params"]
        optional_params = dict(step["optional_params"])
    try:
        mapped = TYPED_CONFIG.map_openai_params(
            non_default_params=non_default_params,
            optional_params=optional_params,
            model=model,
            drop_params=False,
        )
    except ValueError:
        # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L867
        return {"error": {"status": 500}}
    except BadRequestError as error:
        return {"error": {"status": error.status_code}}
    if projection is None:
        return mapped
    return mapped[projection]


def run(input_value: Input) -> Json:
    # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L390
    litellm.enable_gemini_default_thinking_level_low = input_value["settings"][
        "default_thinking_level_low"
    ]
    for name, supported in input_value["catalog"].items():
        entry: JsonObject = {
            "litellm_provider": "vertex_ai-language-models",
            "mode": "chat",
            "supports_minimal_reasoning_effort": supported,
        }
        model_cost: JsonObject = {name: entry}
        litellm.register_model(model_cost)
    return {"results": [answer(step) for step in input_value["steps"]]}
