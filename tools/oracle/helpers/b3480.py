from typing import Literal, TypedDict

from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.google_genai.main import GenerateContentRequestDict, GenerateContentResponse
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, Field, JsonValue, TypeAdapter


class ConstructCall(TypedDict):
    record: Literal["response", "request"]
    fields: dict[str, JsonValue]


class TopLevelCall(TypedDict):
    provider: Literal["studio", "vertex"]
    model: str


class ErrorCall(TypedDict):
    message: str
    status_code: int
    headers: dict[str, JsonValue]


class Input(TypedDict):
    constructs: list[ConstructCall]
    top_level: list[TopLevelCall]
    errors: list[ErrorCall]


class ErrorRecord(BaseModel):
    status_code: int
    message: str
    headers: JsonValue


class HiddenRecord(BaseModel):
    hidden_params: dict[str, JsonValue] = Field(alias="_hidden_params")


JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
INPUT: TypeAdapter[Input] = TypeAdapter(Input)
PROVIDERS: dict[str, LlmProviders] = {
    "studio": LlmProviders.GEMINI,
    "vertex": LlmProviders.VERTEX_AI,
}


def construct(call: ConstructCall) -> JsonValue:
    fields = call["fields"]
    if call["record"] == "response":
        response = GenerateContentResponse(**fields)
        wire = OBJECT.validate_json(response.model_dump_json())
        # deviation: betterlite keeps _hidden_params off the wire (docs/deviations/B3480.yaml)
        wire.pop("_hidden_params", None)
        hidden = HiddenRecord.model_validate(response, from_attributes=True)
        return {"value": wire, "hidden_params": hidden.hidden_params}
    request = GenerateContentRequestDict(**fields)
    return {"value": dict(request)}


def top_level(call: TopLevelCall) -> JsonValue:
    selected = ProviderConfigManager.get_provider_google_genai_generate_content_config(
        call["model"], PROVIDERS[call["provider"]]
    )
    if selected is None:
        raise ValueError(f"no native GenAI config for {call['model']}")
    fields = list(selected.get_generate_content_request_top_level_fields())
    answer: dict[str, JsonValue] = {"provider": call["provider"], "model": call["model"]}
    answer["fields"] = JSON.validate_python(fields)
    return answer


def error(call: ErrorCall) -> JsonValue:
    # get_error_class returns this constructor's value; its bare dict annotation is unknown
    raised = BaseLLMException(
        status_code=call["status_code"],
        message=call["message"],
        headers=call["headers"],
    )
    record = ErrorRecord.model_validate(raised, from_attributes=True)
    answer: dict[str, JsonValue] = {
        "status_code": record.status_code,
        "message": record.message,
    }
    answer["headers"] = record.headers
    return answer


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    answers: list[JsonValue] = [construct(call) for call in parsed["constructs"]]
    answers.extend(top_level(entry) for entry in parsed["top_level"])
    answers.extend(error(failure) for failure in parsed["errors"])
    return {"calls": answers}
