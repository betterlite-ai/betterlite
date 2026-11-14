from typing import Final, Literal, TypedDict

import litellm
from litellm.llms.vertex_ai.fine_tuning.handler import VertexFineTuningAPI
from litellm.types.llms.openai import FineTuningJobCreate
from litellm.types.llms.vertex_ai import ResponseTuningJob
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

API: Final = VertexFineTuningAPI()

JSON: Final = TypeAdapter(JsonValue)


class RequestCall(TypedDict):
    data: dict[str, JsonValue]
    original_hyperparameters: dict[str, JsonValue]


class ResponseCall(TypedDict):
    response: ResponseTuningJob


class RequestInput(TypedDict):
    operation: Literal["request"]
    calls: list[RequestCall]


class ResponseInput(TypedDict):
    operation: Literal["response"]
    calls: list[ResponseCall]


class Job(TypedDict):
    id: str
    created_at: int
    error: dict[str, JsonValue] | None
    fine_tuned_model: str | None
    finished_at: int | None
    hyperparameters: dict[str, JsonValue]
    model: str
    object: str
    organization_id: str
    result_files: list[str]
    seed: int | None
    status: str
    trained_tokens: int | None
    training_file: str
    validation_file: str | None
    estimated_finish: int | None
    integrations: list[dict[str, JsonValue]]
    metadata: dict[str, str] | None
    method: dict[str, JsonValue] | None


def request_outcome(call: RequestCall) -> JsonValue:
    try:
        data = TypeAdapter(FineTuningJobCreate).validate_python(call["data"])
        job = API.convert_openai_request_to_vertex(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/fine_tuning/handler.py#L52-L56
            data, original_hyperparameters=call["original_hyperparameters"]
        )
    except (KeyError, TypeError, ValueError):
        return {"ok": False}
    return {"ok": True, "job": JSON.validate_python(job)}


def response_outcome(call: ResponseCall) -> JsonValue:
    try:
        job = API.convert_vertex_response_to_open_ai_response(call["response"])
        fields = TypeAdapter(Job).validate_json(job.model_dump_json())
    except (KeyError, TypeError, ValueError):
        return {"ok": False}
    return {"ok": True, "job": JSON.validate_python(fields)}


def run(input_value: RequestInput | ResponseInput) -> JsonValue:
    if input_value["operation"] == "request":
        return {"calls": [request_outcome(call) for call in input_value["calls"]]}
    return {"calls": [response_outcome(call) for call in input_value["calls"]]}
