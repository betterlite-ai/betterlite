import os
from collections.abc import Mapping
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
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
    DecisionsIRChoiceOption,
    DecisionsIRChoiceProbability,
    DecisionsIRChoiceQuestion,
    DecisionsIRMessages,
    DecisionsIRPredicateAnswer,
    DecisionsIRPredicateQuestion,
    DecisionsIRQuestion,
    DecisionsIRRefusal,
    DecisionsIRRequest,
    DecisionsIRResponse,
    DecisionsIRScoreAnswer,
    DecisionsIRScoreLevel,
    DecisionsIRScoreProbability,
    DecisionsIRScoreQuestion,
    DecisionsIRState,
    DecisionsIRUsage,
    DecisionsJSON,
    NoulCriteria,
    OpenAIDecisionInputMessage,
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


def as_mapping(value: Json) -> Mapping[str, object]:
    if value is None:
        return {}
    return as_object(value, "extra")


def token_count(fields: dict[str, Json], name: str) -> int:
    return cast(int, fields.get(name, 0))


def ir_choice_option(value: Json) -> DecisionsIRChoiceOption:
    fields = as_object(value, "choice option")
    return DecisionsIRChoiceOption(
        value=cast(str | bool, fields["value"]),
        description=cast(DecisionsJSON | None, fields.get("description")),
    )


def ir_score_level(value: Json) -> DecisionsIRScoreLevel:
    fields = as_object(value, "score level")
    return DecisionsIRScoreLevel(
        label=cast(DecisionsJSON, fields["label"]),
        description=cast(str | None, fields.get("description")),
    )


def ir_question_from(value: Json) -> DecisionsIRQuestion:
    fields = as_object(value, "question")
    if "predicate" in fields:
        body = as_object(fields["predicate"], "predicate question")
        return DecisionsIRPredicateQuestion(
            name=cast(str | None, body.get("name")),
            instructions=cast(DecisionsJSON | None, body.get("instructions")),
            criteria=cast(NoulCriteria | None, body.get("criteria")),
            extra=as_mapping(body.get("extra")),
        )
    if "choice" in fields:
        body = as_object(fields["choice"], "choice question")
        choices = as_list(body["choices"], "choice question choices")
        return DecisionsIRChoiceQuestion(
            name=cast(str | None, body.get("name")),
            instructions=cast(DecisionsJSON | None, body.get("instructions")),
            choices=tuple(ir_choice_option(choice) for choice in choices),
            extra=as_mapping(body.get("extra")),
        )
    body = as_object(fields["score"], "score question")
    levels = as_list(body["levels"], "score question levels")
    return DecisionsIRScoreQuestion(
        name=cast(str | None, body.get("name")),
        instructions=cast(DecisionsJSON | None, body.get("instructions")),
        levels=tuple(ir_score_level(level) for level in levels),
        extra=as_mapping(body.get("extra")),
    )


def ir_input_from(value: Json) -> DecisionsIRState | DecisionsIRMessages:
    fields = as_object(value, "request input")
    if "state" in fields:
        return DecisionsIRState(state=cast(DecisionsJSON, fields["state"]))
    messages = as_list(fields["messages"], "request input messages")
    return DecisionsIRMessages(
        messages=tuple(OpenAIDecisionInputMessage.model_validate(message) for message in messages)
    )


def ir_request_from(value: Json) -> DecisionsIRRequest:
    fields = as_object(value, "request")
    questions = as_list(fields["questions"], "request questions")
    return DecisionsIRRequest(
        input=ir_input_from(fields["input"]),
        questions=tuple(ir_question_from(question) for question in questions),
        safety_identifier=cast(str | None, fields.get("safety_identifier")),
    )


def ir_answer_from(value: Json) -> DecisionsIRAnswer:
    if isinstance(value, str):
        if value != "refusal":
            raise ValueError(f"unknown decision answer {value}")
        return DecisionsIRRefusal()
    fields = as_object(value, "answer")
    if "predicate" in fields:
        body = as_object(fields["predicate"], "predicate answer")
        return DecisionsIRPredicateAnswer(
            probability=cast(float, body["probability"]),
            extra=as_mapping(body.get("extra")),
        )
    if "choice" in fields:
        body = as_object(fields["choice"], "choice answer")
        probabilities = as_list(body["probabilities"], "choice answer probabilities")
        return DecisionsIRChoiceAnswer(
            choice=cast(str | bool, body["choice"]),
            confidence=cast(float, body["confidence"]),
            probabilities=tuple(ir_choice_probability(item) for item in probabilities),
            extra=as_mapping(body.get("extra")),
        )
    body = as_object(fields["score"], "score answer")
    probabilities = as_list(body["probabilities"], "score answer probabilities")
    return DecisionsIRScoreAnswer(
        score=cast(float, body["score"]),
        confidence=cast(float, body["confidence"]),
        probabilities=tuple(ir_score_probability(item) for item in probabilities),
        extra=as_mapping(body.get("extra")),
    )


def ir_choice_probability(value: Json) -> DecisionsIRChoiceProbability:
    fields = as_object(value, "choice probability")
    return DecisionsIRChoiceProbability(
        value=cast(str | bool, fields["value"]),
        probability=cast(float, fields["probability"]),
    )


def ir_score_probability(value: Json) -> DecisionsIRScoreProbability:
    fields = as_object(value, "score probability")
    return DecisionsIRScoreProbability(
        value=cast(int, fields["value"]),
        label=cast(DecisionsJSON, fields["label"]),
        probability=cast(float, fields["probability"]),
    )


def ir_response_from(value: Json) -> DecisionsIRResponse:
    fields = as_object(value, "response")
    usage = as_object(fields.get("usage") or {}, "response usage")
    answers = as_list(fields["answers"], "response answers")
    return DecisionsIRResponse(
        model=cast(str | None, fields.get("model")),
        answers=tuple(ir_answer_from(answer) for answer in answers),
        usage=DecisionsIRUsage(
            input_tokens=token_count(usage, "input_tokens"),
            output_tokens=token_count(usage, "output_tokens"),
            cached_tokens=token_count(usage, "cached_tokens"),
            cache_write_tokens=token_count(usage, "cache_write_tokens"),
            reasoning_tokens=token_count(usage, "reasoning_tokens"),
            extra=as_mapping(usage.get("extra")),
        ),
        extra=as_mapping(fields.get("extra")),
    )


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


def ir_to_request(call: dict[str, Json]) -> Json:
    request = ir_request_from(call["request"])
    result = ir_to_openai_request(cast(str, call["model"]), request)
    if isinstance(result, UnsupportedDecisionsRequest):
        return failed()
    return succeeded(cast(Json, result))


def request_roundtrip(call: dict[str, Json]) -> Json:
    body = OpenAIDecisionRequestBody.model_validate(call["request"])
    result = ir_to_openai_request(cast(str, call["model"]), openai_request_to_ir(body))
    if isinstance(result, UnsupportedDecisionsRequest):
        return failed()
    return succeeded(cast(Json, result))


def response_to_ir(call: dict[str, Json]) -> Json:
    body = OpenAIDecisionRequestBody.model_validate(call["request"])
    response = OpenAIDecisionResponse.model_validate(call["response"])
    converted = openai_response_to_ir(response, openai_request_to_ir(body))
    return succeeded(ir_response_value(converted))


def ir_to_response(call: dict[str, Json]) -> Json:
    request = ir_request_from(call["request"])
    response = ir_response_from(call["response"])
    try:
        converted = ir_to_openai_response(
            response, request, cast(str, call["requested_model"])
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
        "ir_to_request": [ir_to_request(call) for call in calls(input_value, "ir_to_request")],
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
