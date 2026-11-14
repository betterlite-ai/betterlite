import logging
from logging.handlers import MemoryHandler
from typing import Literal, Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import VertexGeminiConfig
from openai import BadRequestError
from pydantic import JsonValue as Json

JsonObject: TypeAlias = dict[str, Json]


class ParamMapper(Protocol):
    def map_openai_params(
        self,
        *,
        non_default_params: JsonObject,
        optional_params: JsonObject,
        model: str,
        drop_params: bool,
    ) -> JsonObject: ...

    def map_special_auth_params(
        self,
        non_default_params: JsonObject,
        optional_params: JsonObject,
    ) -> JsonObject: ...


TYPED_CONFIG: ParamMapper = VertexGeminiConfig()


class Step(TypedDict):
    op: Literal["options", "auth"]
    model: str
    non_default_params: JsonObject
    optional_params: JsonObject


class Input(TypedDict):
    safety_settings: dict[str, Json] | None
    steps: list[Step]


def answer(step: Step) -> Json:
    if step["op"] == "auth":
        return {
            "options": TYPED_CONFIG.map_special_auth_params(
                non_default_params=dict(step["non_default_params"]),
                optional_params=dict(step["optional_params"]),
            ),
            "warnings": 0,
        }
    logger = logging.getLogger("LiteLLM")
    handler = MemoryHandler(capacity=1000, flushLevel=logging.CRITICAL)
    handler.setLevel(logging.WARNING)
    logger.addHandler(handler)
    try:
        mapped = TYPED_CONFIG.map_openai_params(
            non_default_params=dict(step["non_default_params"]),
            optional_params=dict(step["optional_params"]),
            model=step["model"],
            drop_params=False,
        )
    except BadRequestError as error:
        return {"error": {"status": error.status_code}}
    except ValueError:
        # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L867
        return {"error": {"status": 500}}
    finally:
        logger.removeHandler(handler)
    return {"options": mapped, "warnings": len(handler.buffer)}


def run(input_value: Input) -> Json:
    # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L598
    litellm.vertex_ai_safety_settings = input_value["safety_settings"]
    try:
        return {"results": [answer(step) for step in input_value["steps"]]}
    finally:
        litellm.vertex_ai_safety_settings = None
