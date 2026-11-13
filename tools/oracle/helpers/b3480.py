from typing import Literal, TypedDict

from litellm.types.google_genai.main import GenerateContentRequestDict, GenerateContentResponse
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, JsonValue, TypeAdapter


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


JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
INPUT: TypeAdapter[Input] = TypeAdapter(Input)
PROVIDERS: dict[str, LlmProviders] = {
    "studio": LlmProviders.GEMINI,
    "vertex": LlmProviders.VERTEX_AI,
}


def construct(call: ConstructCall) -> JsonValue:
    fields = call["fields"]
    if call["record"] == "response":
        response = GenerateContentResponse(**fields)
        dumped = response.model_dump_json()
        return {"value": JSON.validate_json(dumped)}
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
    selected = ProviderConfigManager.get_provider_google_genai_generate_content_config(
        "gemini-2.5-flash", LlmProviders.GEMINI
    )
    if selected is None:
        raise ValueError("no native GenAI config for gemini-2.5-flash")
    raised = selected.get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/google_genai/transformation.py#L198-L216
        error_message=call["message"],
        status_code=call["status_code"],
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
