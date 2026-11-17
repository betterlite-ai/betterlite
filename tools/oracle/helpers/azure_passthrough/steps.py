from typing import Final

from litellm.types.llms.openai import ResponsesTerminalEvent
from pydantic import BaseModel, Field, TypeAdapter
from pydantic import JsonValue as Json

PASSTHROUGH_CALL_TYPE: Final = "allm_passthrough_route"
NONSTREAM_URL: Final = "https://b548.test/openai/deployments/d/chat/completions"
STREAM_MODEL: Final = "gpt-4o"
ENVIRONMENT: Final = ("AZURE_API_BASE", "AZURE_API_KEY", "AZURE_OPENAI_API_KEY", "AZURE_AD_TOKEN")

JSON: TypeAdapter[Json] = TypeAdapter(Json)
FUNCTION: TypeAdapter[str] = TypeAdapter(str)
ADDITIONAL: TypeAdapter[dict[str, Json] | None] = TypeAdapter(dict[str, Json] | None)
TERMINAL: TypeAdapter[ResponsesTerminalEvent] = TypeAdapter(ResponsesTerminalEvent)


class InferenceStep(BaseModel):
    endpoint: str
    request_data: dict[str, Json]


class RouterStep(BaseModel):
    endpoint: str
    router_models: list[str]


class DeploymentStep(BaseModel):
    endpoint: str
    model_group: str
    served_models: list[str]


class UrlStep(BaseModel):
    model: str
    endpoint: str
    litellm_params: dict[str, Json] = Field(default_factory=dict)
    request_query_params: dict[str, Json] | None = None
    api_base: str | None = None
    env_base: str | None = None


class StreamStep(BaseModel):
    endpoint: str
    chunks: list[str]


class RelayStep(BaseModel):
    endpoint: str
    model: str
    chunks: list[str]
    model_call_details: dict[str, Json] | None = None


class NonstreamStep(BaseModel):
    endpoint: str
    model: str
    status: int
    headers: dict[str, str] = Field(default_factory=dict)
    body: dict[str, Json]
    request_data: dict[str, Json] = Field(default_factory=dict)


class KeyStep(BaseModel):
    api_key: str | None = None
    env_key: str | None = None


class ValidationStep(BaseModel):
    headers: dict[str, str] = Field(default_factory=dict)
    api_key: str | None = None
    params_key: str | None = None
    configured_key: str | None = None


INFERENCE_STEPS: TypeAdapter[list[InferenceStep]] = TypeAdapter(list[InferenceStep])
ROUTER_STEPS: TypeAdapter[list[RouterStep]] = TypeAdapter(list[RouterStep])
DEPLOYMENT_STEPS: TypeAdapter[list[DeploymentStep]] = TypeAdapter(list[DeploymentStep])
URL_STEPS: TypeAdapter[list[UrlStep]] = TypeAdapter(list[UrlStep])
REMOVALS: TypeAdapter[list[str]] = TypeAdapter(list[str])
STREAM_STEPS: TypeAdapter[list[StreamStep]] = TypeAdapter(list[StreamStep])
RELAY_STEPS: TypeAdapter[list[RelayStep]] = TypeAdapter(list[RelayStep])
NONSTREAM_STEPS: TypeAdapter[list[NonstreamStep]] = TypeAdapter(list[NonstreamStep])
KEY_STEPS: TypeAdapter[list[KeyStep]] = TypeAdapter(list[KeyStep])
VALIDATION_STEPS: TypeAdapter[list[ValidationStep]] = TypeAdapter(list[ValidationStep])


def terminal_view(serialized: str) -> Json:
    event = TERMINAL.validate_json(serialized)
    return {
        "class": type(event).__name__,
        "id": event.response.id,
        "created_at": event.response.created_at,
    }
