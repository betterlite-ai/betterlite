import os

from litellm.llms.openai.decisions.transformation import (
    OpenAIDecisionsConfig,
    ir_to_openai_response,
    openai_request_to_ir,
)
from litellm.types.decisions import OpenAIDecisionRequestBody
from pydantic import JsonValue, TypeAdapter

from .support import JSON, Input

DECISION: TypeAdapter[OpenAIDecisionRequestBody] = TypeAdapter(OpenAIDecisionRequestBody)
BODY: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


def decisions_answer(input_value: Input) -> JsonValue:
    # Nothing explicit or global is set, so the case supplies the variables the leaf reads.
    previous = dict(os.environ)
    os.environ.clear()
    os.environ.update(input_value["environment"])
    try:
        config = OpenAIDecisionsConfig()
        ir = openai_request_to_ir(DECISION.validate_python(input_value["decision"]))
        payload: dict[str, JsonValue] = {
            "api_base": config.resolve_api_base(input_value["api_base"]),
            "api_key": config.resolve_api_key(input_value["api_key"]),
        }
        if input_value["operation"] == "request":
            wire = config.transform_decisions_request(input_value["model"], ir, "openai")
            payload["request"] = BODY.validate_python(wire)
        else:
            response = config.parse_response(JSON.validate_json(input_value["payload"]), ir)
            projected = ir_to_openai_response(response, ir, input_value["model"])
            payload["response"] = BODY.validate_json(projected.model_dump_json())
        return payload
    finally:
        os.environ.clear()
        os.environ.update(previous)
