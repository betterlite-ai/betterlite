import io
import json
import os
from contextlib import redirect_stdout
from datetime import UTC, datetime
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import litellm
from litellm.exceptions import AuthenticationError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.github_copilot.responses.transformation import (
    GithubCopilotResponsesAPIConfig,
    github_copilot_supports_responses_api,
)
from litellm.types.llms.openai import ResponsesAPIStreamingResponse
from litellm.types.router import GenericLiteLLMParams
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import JsonValue, TypeAdapter

KEY_EXPIRES_AT: Final = 4102444800
ENVIRONMENT_NAMES: Final = ("GITHUB_COPILOT_TOKEN_DIR", "GITHUB_COPILOT_API_BASE")
MODEL: Final = "gpt-5"

litellm.suppress_debug_info = True


class CopilotResponses(Protocol):
    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        litellm_params: GenericLiteLLMParams | None,
        /,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue], /
    ) -> str: ...

    def transform_streaming_response(
        self, model: str, parsed_chunk: dict[str, JsonValue], logging_obj: Logging, /
    ) -> ResponsesAPIStreamingResponse: ...

    # ast-grep-ignore: no-long-names
    def transform_responses_api_request(
        self,
        model: str,
        input_value: str | ResponseInputParam,
        optional_params: dict[str, JsonValue],
        litellm_params: GenericLiteLLMParams,
        headers: dict[str, str],
        /,
    ) -> dict[str, JsonValue]: ...

    def supports_native_websocket(self) -> bool: ...


class ItemView(TypedDict, total=False):
    id: str | None


class EventView(TypedDict, total=False):
    item_id: str | None
    item: ItemView | None


EVENT: Final[TypeAdapter[EventView]] = TypeAdapter(EventView)


class ParamsInput(TypedDict, total=False):
    input: JsonValue
    model_info: dict[str, JsonValue]


class SupportsStep(TypedDict):
    step: Literal["supports"]
    model: str
    models: dict[str, dict[str, JsonValue]]


class ReasoningStep(TypedDict):
    step: Literal["reasoning"]
    input: str | ResponseInputParam


class StreamStep(TypedDict):
    step: Literal["stream"]
    events: list[dict[str, JsonValue]]


class AuthStep(TypedDict):
    step: Literal["auth"]
    copilot_key: str | None
    headers: dict[str, str]
    params: ParamsInput | None


class UrlStep(TypedDict):
    step: Literal["url"]
    endpoints_api: str | None
    api_base: str | None
    env_api_base: str | None


class CapabilityStep(TypedDict):
    step: Literal["capability"]


Step: TypeAlias = (
    SupportsStep | ReasoningStep | StreamStep | AuthStep | UrlStep | CapabilityStep
)


class Input(TypedDict):
    settings: dict[str, str]
    steps: list[Step]


def logging_object() -> Logging:
    return Logging(
        model=MODEL,
        messages=None,
        stream=False,
        call_type="responses",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def clear_environment() -> None:
    for name in ENVIRONMENT_NAMES:
        os.environ.pop(name, None)


def copilot_config(
    root: str, token: str | None, endpoints_api: str | None
) -> CopilotResponses:
    key_file: dict[str, JsonValue] = {
        "token": token,
        "expires_at": KEY_EXPIRES_AT,
        "endpoints": {"api": endpoints_api},
    }
    _ = (Path(root) / "api-key.json").write_text(json.dumps(key_file))
    os.environ["GITHUB_COPILOT_TOKEN_DIR"] = root
    return GithubCopilotResponsesAPIConfig()


def supports_value(step: SupportsStep) -> JsonValue:
    with redirect_stdout(io.StringIO()):
        litellm.register_model(step["models"])
    return github_copilot_supports_responses_api(step["model"])


def reasoning_value(step: ReasoningStep) -> JsonValue:
    with TemporaryDirectory() as root:
        config = copilot_config(root, "unused-key", None)
        body = config.transform_responses_api_request(
            MODEL, step["input"], {}, GenericLiteLLMParams(), {}
        )
    return body["input"]


def stream_value(step: StreamStep) -> JsonValue:
    with TemporaryDirectory() as root:
        config = copilot_config(root, "unused-key", None)
        projected: list[JsonValue] = []
        for chunk in step["events"]:
            event = config.transform_streaming_response(MODEL, chunk, logging_object())
            view = EVENT.validate_json(event.model_dump_json())
            item = view.get("item")
            projected.append(
                {
                    "item_id": view.get("item_id"),
                    "nested_id": None if item is None else item.get("id"),
                }
            )
    return projected


def auth_value(step: AuthStep) -> JsonValue:
    params = step["params"]
    litellm_params = (
        None if params is None else GenericLiteLLMParams.model_validate(params)
    )
    with TemporaryDirectory() as root:
        config = copilot_config(root, step["copilot_key"], None)
        try:
            headers = config.validate_environment(step["headers"], MODEL, litellm_params)
        except AuthenticationError as error:
            return {"raised": error.status_code}
    shown: dict[str, JsonValue] = {
        name: value for name, value in headers.items() if name != "x-request-id"
    }
    return {"headers": shown, "request_id": "x-request-id" in headers}


def url_value(step: UrlStep) -> JsonValue:
    clear_environment()
    if step["env_api_base"] is not None:
        os.environ["GITHUB_COPILOT_API_BASE"] = step["env_api_base"]
    with TemporaryDirectory() as root:
        config = copilot_config(root, "unused-key", step["endpoints_api"])
        return config.get_complete_url(step["api_base"], {})


def capability_value() -> JsonValue:
    with TemporaryDirectory() as root:
        config = copilot_config(root, "unused-key", None)
        return {"native_websocket": config.supports_native_websocket()}


def step_value(step: Step) -> JsonValue:
    match step["step"]:
        case "supports":
            return supports_value(step)
        case "reasoning":
            return reasoning_value(step)
        case "stream":
            return stream_value(step)
        case "auth":
            return auth_value(step)
        case "url":
            return url_value(step)
        case "capability":
            return capability_value()


def run(input_value: Input) -> JsonValue:
    clear_environment()
    try:
        return [step_value(step) for step in input_value["steps"]]
    finally:
        clear_environment()
