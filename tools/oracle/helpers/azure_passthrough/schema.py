from typing import Final, Literal, Required, TypedDict

from litellm.types.llms.openai import ResponsesTerminalEvent
from pydantic import JsonValue as Json, TypeAdapter

PASSTHROUGH_CALL_TYPE: Final = "allm_passthrough_route"
NONSTREAM_URL: Final = "https://b548.test/openai/deployments/d/chat/completions"
STREAM_MODEL: Final = "gpt-4o"
ENVIRONMENT: Final = ("AZURE_API_BASE", "AZURE_API_KEY", "AZURE_OPENAI_API_KEY", "AZURE_AD_TOKEN")

JSON: TypeAdapter[Json] = TypeAdapter(Json)
HEADERS: TypeAdapter[dict[str, str]] = TypeAdapter(dict[str, str])
ADDITIONAL: TypeAdapter[dict[str, Json] | None] = TypeAdapter(dict[str, Json] | None)
TERMINAL: TypeAdapter[ResponsesTerminalEvent] = TypeAdapter(ResponsesTerminalEvent)


def terminal_view(serialized: str) -> Json:
    event = TERMINAL.validate_json(serialized)
    return {
        "class": type(event).__name__,
        "id": event.response.id,
        "created_at": event.response.created_at,
    }


class Step(TypedDict, total=False):
    endpoint: Required[str]
    request_data: dict[str, Json]
    router_models: list[str]
    model_group: str
    served_models: list[str]
    model: str
    chunks: list[str]
    status: int
    headers: dict[str, str]
    body: dict[str, Json]
    model_call_details: dict[str, Json] | None
    litellm_params: dict[str, Json]
    request_query_params: dict[str, Json] | None
    api_base: str | None
    env_base: str | None
    api_key: str | None
    env_key: str | None
    params_key: str | None
    configured_key: str | None


class Input(TypedDict, total=False):
    function: Required[
        Literal[
            "body_model_inference_endpoint",
            "router_model_in_path",
            "foreign_deployment",
            "api_version_removal",
            "responses_stream_log",
            "messages_relay_log",
            "nonstream_usage",
            "passthrough_auth_inputs",
        ]
    ]
    inference_steps: list[Step]
    router_steps: list[Step]
    deployment_steps: list[Step]
    removals: list[str]
    url_steps: list[Step]
    stream_steps: list[Step]
    relay_steps: list[Step]
    nonstream_steps: list[Step]
    key_steps: list[Step]
    validation_steps: list[Step]
