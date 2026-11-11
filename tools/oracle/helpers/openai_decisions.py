import os
from collections.abc import Mapping
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.base_llm.decisions.transformation import (
    systemone_request_to_ir,
    systemone_response_to_ir,
)
from litellm.llms.openai.decisions.transformation import (
    OpenAIDecisionsConfig,
    ir_to_openai_request,
    ir_to_openai_response,
    openai_request_to_ir,
    openai_response_to_ir,
)
from litellm.types.decisions import (
    DecisionsIRAnswer,
    DecisionsIRChoiceAnswer,
    DecisionsIRPredicateAnswer,
    DecisionsIRRefusal,
    DecisionsIRResponse,
    DecisionsRequestBody,
    DecisionsResponse,
    OpenAIDecisionRequestBody,
    OpenAIDecisionResponse,
    UnsupportedDecisionsRequest,
)
from pydantic import ValidationError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def as_object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def as_list(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def ir_answer_value(answer: DecisionsIRAnswer) -> Json:
    if isinstance(answer, DecisionsIRRefusal):
        return {"refusal": {}}
    if isinstance(answer, DecisionsIRPredicateAnswer):
        return {
            "predicate": {
                "probability": answer.probability,
                "extra": cast(Json, dict(answer.extra)),
            }
        }
    if isinstance(answer, DecisionsIRChoiceAnswer):
        return {
            "choice": {
                "choice": answer.choice,
                "confidence": answer.confidence,
                "probabilities": [
                    {"value": item.value, "probability": item.probability}
                    for item in answer.probabilities
                ],
                "extra": cast(Json, dict(answer.extra)),
            }
        }
    return {
        "score": {
            "score": answer.score,
            "confidence": answer.confidence,
            "probabilities": [
                {
                    "value": item.value,
                    "label": cast(Json, item.label),
                    "probability": item.probability,
                }
                for item in answer.probabilities
            ],
            "extra": cast(Json, dict(answer.extra)),
        }
    }


def ir_response_value(response: DecisionsIRResponse) -> Json:
    return {
        "model": response.model,
        "answers": [ir_answer_value(answer) for answer in response.answers],
        "usage": {
            "input_tokens": response.usage.input_tokens,
            "output_tokens": response.usage.output_tokens,
            "cached_tokens": response.usage.cached_tokens,
            "cache_write_tokens": response.usage.cache_write_tokens,
            "reasoning_tokens": response.usage.reasoning_tokens,
            "extra": cast(Json, dict(response.usage.extra)),
        },
        "extra": cast(Json, dict(response.extra)),
    }


def failed() -> Json:
    return {"ok": False}


def succeeded(value: Json) -> Json:
    return {"ok": True, "value": value}


def request_value(outcome: Mapping[str, object] | UnsupportedDecisionsRequest) -> Json:
    if isinstance(outcome, UnsupportedDecisionsRequest):
        return failed()
    return succeeded(cast(Json, outcome))


def systemone_to_request(call: dict[str, Json]) -> Json:
    request = systemone_request_to_ir(DecisionsRequestBody.model_validate(call["request"]))
    return request_value(ir_to_openai_request(cast(str, call["model"]), request))


def request_roundtrip(call: dict[str, Json]) -> Json:
    try:
        body = OpenAIDecisionRequestBody.model_validate(call["request"])
    except ValidationError:
        return failed()
    return request_value(ir_to_openai_request(cast(str, call["model"]), openai_request_to_ir(body)))


def response_to_ir(call: dict[str, Json]) -> Json:
    body = OpenAIDecisionRequestBody.model_validate(call["request"])
    response = OpenAIDecisionResponse.model_validate(call["response"])
    converted = openai_response_to_ir(response, openai_request_to_ir(body))
    return succeeded(ir_response_value(converted))


def source_response(source: dict[str, Json]) -> DecisionsIRResponse:
    if "openai" in source:
        body = as_object(source["openai"], "openai source")
        request = openai_request_to_ir(
            OpenAIDecisionRequestBody.model_validate(body["request"])
        )
        response = OpenAIDecisionResponse.model_validate(body["response"])
        return openai_response_to_ir(response, request)
    body = as_object(source["systemone"], "systemone source")
    request = systemone_request_to_ir(DecisionsRequestBody.model_validate(body["request"]))
    response = DecisionsResponse.model_validate(body["response"])
    return systemone_response_to_ir(response, request)


def ir_to_response(call: dict[str, Json]) -> Json:
    source = source_response(as_object(call["source"], "source"))
    target = openai_request_to_ir(
        OpenAIDecisionRequestBody.model_validate(call["target"])
    )
    try:
        converted = ir_to_openai_response(
            source, target, cast(str, call["requested_model"])
        )
    except ValueError:
        return failed()
    return succeeded(cast(Json, converted.model_dump(mode="json")))


def parse_response(call: dict[str, Json]) -> Json:
    body = OpenAIDecisionRequestBody.model_validate(call["request"])
    config = OpenAIDecisionsConfig()
    try:
        converted = config.parse_response(  # pyright: ignore[reportUnknownMemberType]
            call["payload"], openai_request_to_ir(body)
        )
    except ValidationError:
        return failed()
    return succeeded(ir_response_value(converted))


def settings(call: dict[str, Json]) -> Json:
    env = cast(dict[str, str], call.get("env") or {})
    globals_value = as_object(call.get("globals") or {}, "globals")
    with (
        patch.dict(os.environ, env, clear=True),
        patch.object(litellm, "api_base", cast(str | None, globals_value.get("api_base"))),
        patch.object(litellm, "api_key", cast(str | None, globals_value.get("api_key"))),
        patch.object(litellm, "openai_key", cast(str | None, globals_value.get("openai_key"))),
    ):
        config = OpenAIDecisionsConfig()
        api_base = config.resolve_api_base(cast(str | None, call.get("api_base")))
        api_key = config.resolve_api_key(cast(str | None, call.get("api_key")))
    return {"api_base": api_base, "api_key": api_key}


def calls(input_value: dict[str, Json], name: str) -> list[dict[str, Json]]:
    return [as_object(call, name) for call in as_list(input_value.get(name, []), name)]


def run(input_value: dict[str, Json]) -> Json:
    return {
        "systemone_to_request": [
            systemone_to_request(call) for call in calls(input_value, "systemone_to_request")
        ],
        "request_roundtrip": [
            request_roundtrip(call) for call in calls(input_value, "request_roundtrip")
        ],
        "response_to_ir": [
            response_to_ir(call) for call in calls(input_value, "response_to_ir")
        ],
        "ir_to_response": [
            ir_to_response(call) for call in calls(input_value, "ir_to_response")
        ],
        "parse_response": [
            parse_response(call) for call in calls(input_value, "parse_response")
        ],
        "settings": [settings(call) for call in calls(input_value, "settings")],
    }
