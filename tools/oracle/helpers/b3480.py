from typing import Literal, TypedDict

from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.google_genai.main import (
    GenerateContentConfigDict,
    GenerateContentRequestDict,
    GenerateContentResponse,
)
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter


class ConstructCall(TypedDict):
    record: Literal["response", "config", "request"]
    fields: dict[str, JsonValue]


class TopLevelCall(TypedDict):
    provider: Literal["studio", "vertex"]
    model: str


class ErrorCall(TypedDict):
    message: str
    status_code: int
    headers: dict[str, JsonValue]


class Input(TypedDict, total=False):
    constructs: list[ConstructCall]
    top_level: list[TopLevelCall]
    errors: list[ErrorCall]


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
        return {"value": response.model_dump()}
    if call["record"] == "config":
        config = GenerateContentConfigDict(**fields)
        return {"value": dict(config)}
    request = GenerateContentRequestDict(**fields)
    return {"value": dict(request)}


def top_level(call: TopLevelCall) -> JsonValue:
    selected = ProviderConfigManager.get_provider_google_genai_generate_content_config(
        call["model"], PROVIDERS[call["provider"]]
    )
    if selected is None:
        raise ValueError(f"no native GenAI config for {call['model']}")
    fields = list(selected.get_generate_content_request_top_level_fields())
    return {"provider": call["provider"], "model": call["model"], "fields": fields}


def error(call: ErrorCall) -> JsonValue:
    selected = ProviderConfigManager.get_provider_google_genai_generate_content_config(
        "gemini-2.5-flash", LlmProviders.GEMINI
    )
    if selected is None:
        raise ValueError("no native GenAI config for gemini-2.5-flash")
    try:
        raise selected.get_error_class(
            error_message=call["message"],
            status_code=call["status_code"],
            headers=call["headers"],
        )
    except BaseLLMException as raised:
        return {
            "status_code": raised.status_code,
            "message": raised.message,
            "headers": JSON.validate_python(raised.headers),
        }


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    answers: list[JsonValue] = [construct(call) for call in parsed["constructs"]]
    answers.extend(top_level(entry) for entry in parsed["top_level"])
    answers.extend(error(failure) for failure in parsed["errors"])
    return {"calls": answers}
