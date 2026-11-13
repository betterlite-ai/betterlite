import asyncio
from typing import Literal, TypedDict

import httpx
from litellm.exceptions import GuardrailRaisedException
from litellm.proxy.guardrails.guardrail_hooks.singulr.singulr import SingulrGuardrail
from litellm.types.utils import GenericGuardrailAPIInputs
from pydantic import JsonValue, TypeAdapter

REPORTED_HEADERS = (
    "content-type",
    "x-singulr-gateway-token",
    "x-singulr-enforcement-entity-id",
    "x-singulr-guardrail-id",
)

JSON_ADAPTER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
INPUTS_ADAPTER: TypeAdapter[GenericGuardrailAPIInputs] = TypeAdapter(GenericGuardrailAPIInputs)


class Settings(TypedDict):
    api_key: str | None
    api_base: str | None
    application_id: str | None
    guardrail_id: str | None
    block_on_error: bool | None
    timeout: float | None


class Answer(TypedDict):
    status: int
    body: JsonValue


class Scenario(TypedDict):
    name: str
    settings: Settings
    inputs: dict[str, JsonValue]
    request_data: dict[str, JsonValue]
    input_type: Literal["request", "response"]
    answer: Answer | None


class Input(TypedDict):
    scenarios: list[Scenario]


class Seen(TypedDict):
    path: str
    headers: dict[str, str | None]
    body: JsonValue


class ScenarioOutput(TypedDict):
    name: str
    outcome: Literal["allowed", "blocked", "error"]
    status: int | None
    calls: list[Seen] | None
    inputs: JsonValue


class Output(TypedDict):
    supported_event_hooks: list[str]
    scenarios: list[ScenarioOutput]


def reported_headers(headers: httpx.Headers) -> dict[str, str | None]:
    seen = dict(headers)
    return {name: seen.get(name) for name in REPORTED_HEADERS}


class Script:
    def __init__(self, answer: Answer | None) -> None:
        self.answer = answer
        self.calls: list[Seen] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.calls.append(
            {
                "path": request.url.path,
                "headers": reported_headers(request.headers),
                "body": JSON_ADAPTER.validate_json(request.content),
            }
        )
        if self.answer is None:
            raise httpx.ConnectError("the scripted endpoint is unreachable", request=request)
        return httpx.Response(
            status_code=self.answer["status"],
            json=self.answer["body"],
            request=request,
        )


def default_settings() -> Settings:
    return {
        "api_key": None,
        "api_base": None,
        "application_id": None,
        "guardrail_id": None,
        "block_on_error": None,
        "timeout": None,
    }


def build_guardrail(settings: Settings) -> SingulrGuardrail:
    return SingulrGuardrail(
        singulr_api_key=settings["api_key"],
        singulr_api_base=settings["api_base"],
        singulr_application_id=settings["application_id"],
        singulr_guardrail_id=settings["guardrail_id"],
        block_on_error=settings["block_on_error"],
        timeout=settings["timeout"],
    )


def supported_hooks() -> list[str]:
    hooks = build_guardrail(default_settings()).supported_event_hooks
    if hooks is None:
        raise RuntimeError("SingulrGuardrail did not install its supported event hooks")
    return [hook.value for hook in hooks]


def run_scenario(scenario: Scenario) -> ScenarioOutput:
    guardrail = build_guardrail(scenario["settings"])
    script = Script(scenario["answer"])
    guardrail.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(script))
    try:
        returned = asyncio.run(
            guardrail.apply_guardrail(
                INPUTS_ADAPTER.validate_python(scenario["inputs"]),
                dict(scenario["request_data"]),
                scenario["input_type"],
            )
        )
    except GuardrailRaisedException as error:
        outcome: Literal["blocked", "error"] = "blocked" if error.blocked_content else "error"
        return {
            "name": scenario["name"],
            "outcome": outcome,
            "status": error.status_code,
            "calls": script.calls if scenario["answer"] is not None else None,
            "inputs": None,
        }
    return {
        "name": scenario["name"],
        "outcome": "allowed",
        "status": None,
        "calls": script.calls,
        "inputs": JSON_ADAPTER.validate_python(returned),
    }


def run(input_value: Input) -> Output:
    return {
        "supported_event_hooks": supported_hooks(),
        "scenarios": [run_scenario(scenario) for scenario in input_value["scenarios"]],
    }
