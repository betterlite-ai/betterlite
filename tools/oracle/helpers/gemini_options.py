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


class OptionsStep(TypedDict):
    op: Literal["options"]
    model: str
    non_default_params: JsonObject
    optional_params: JsonObject


class AuthStep(TypedDict):
    op: Literal["auth"]
    non_default_params: JsonObject
    optional_params: JsonObject


Step: TypeAlias = OptionsStep | AuthStep


class Input(TypedDict):
    safety_settings: dict[str, Json] | None
    steps: list[Step]


def capture_warnings() -> MemoryHandler:
    handler = MemoryHandler(capacity=1000, flushLevel=logging.CRITICAL)
    handler.setLevel(logging.WARNING)
    logging.getLogger("LiteLLM").addHandler(handler)
    return handler


def options_answer(step: OptionsStep) -> Json:
    logger = logging.getLogger("LiteLLM")
    handler = capture_warnings()
    try:
        mapped = TYPED_CONFIG.map_openai_params(
            non_default_params=dict(step["non_default_params"]),
            optional_params=dict(step["optional_params"]),
            model=step["model"],
            drop_params=False,
        )
    except ValueError:
        # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L867
        return {"error": {"status": 500}}
    except BadRequestError as error:
        return {"error": {"status": error.status_code}}
    finally:
        logger.removeHandler(handler)
    return {"options": mapped, "warnings": len(handler.buffer)}


def auth_answer(step: AuthStep) -> Json:
    mapped = TYPED_CONFIG.map_special_auth_params(
        non_default_params=dict(step["non_default_params"]),
        optional_params=dict(step["optional_params"]),
    )
    return {"options": mapped, "warnings": 0}


def run(input_value: Input) -> Json:
    # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L598
    litellm.vertex_ai_safety_settings = input_value["safety_settings"]
    try:
        results: list[Json] = []
        for step in input_value["steps"]:
            if step["op"] == "auth":
                results.append(auth_answer(step))
            else:
                results.append(options_answer(step))
        return {"results": results}
    finally:
        litellm.vertex_ai_safety_settings = None
