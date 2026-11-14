import asyncio
from collections.abc import Awaitable, Callable
from typing import Literal, TypeAlias, TypedDict, cast

import httpx
from litellm.proxy.guardrails.guardrail_hooks.enkryptai import EnkryptAIGuardrails
from pydantic import JsonValue, TypeAdapter


class Settings(TypedDict, total=False):
    api_key: str
    api_base: str
    policy_name: str
    deployment_name: str
    detectors: dict[str, JsonValue]
    block_on_violation: bool


class ConstructCall(TypedDict):
    settings: Settings


class Answer(TypedDict):
    status: int
    body: JsonValue


class DetectCall(TypedDict):
    settings: Settings
    texts: list[str]
    answer: Answer


class ConstructOutput(TypedDict):
    api_base: str
    api_url: str
    policy_name: str | None
    provider: str
    hooks: list[str]


class Headers(TypedDict):
    content_type: str | None
    apikey: str | None
    x_enkrypt_policy: str | None


class DetectBody(TypedDict):
    text: str


class RequestView(TypedDict):
    method: str
    path: str
    headers: Headers
    body: DetectBody


class ResultView(TypedDict, total=False):
    texts: list[str]


class ProcessedView(TypedDict):
    attacks: list[str]
    details: dict[str, JsonValue]


class DetectOutput(TypedDict):
    requests: list[RequestView]
    raised: bool
    statuses: list[str]
    processed: ProcessedView
    result: ResultView | None


class GuardrailRecord(TypedDict):
    guardrail_status: str


class ResponseView(TypedDict, total=False):
    details: dict[str, JsonValue]


Apply: TypeAlias = Callable[
    [dict[str, list[str]], dict[str, JsonValue], Literal["request", "response"], None],
    Awaitable[None],
]


class RecordingTransport:
    def __init__(self, answer: Answer) -> None:
        self.answer = answer
        self.requests: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        return httpx.Response(self.answer["status"], json=self.answer["body"])


def build(settings: Settings) -> EnkryptAIGuardrails:
    return EnkryptAIGuardrails(
        guardrail_name="guardrail",
        api_key=settings.get("api_key"),
        api_base=settings.get("api_base"),
        policy_name=settings.get("policy_name"),
    )


def construct(settings: Settings) -> ConstructOutput:
    guardrail = build(settings)
    hooks = guardrail.supported_event_hooks
    if hooks is None:
        raise ValueError("the guardrail has no supported event hooks")
    return {
        "api_base": TypeAdapter(str).validate_python(guardrail.api_base),
        "api_url": guardrail.api_url,
        "policy_name": guardrail.policy_name,
        "provider": guardrail.guardrail_provider,
        "hooks": TypeAdapter(list[str]).validate_python(hooks),
    }


def view(request: httpx.Request) -> RequestView:
    body = TypeAdapter(DetectBody).validate_json(request.content)
    return {
        "method": request.method,
        "path": request.url.path,
        "headers": {
            "content_type": request.headers.get("content-type"),
            "apikey": request.headers.get("apikey"),
            "x_enkrypt_policy": request.headers.get("x-enkrypt-policy"),
        },
        "body": body,
    }


def statuses(request_data: dict[str, JsonValue]) -> list[str]:
    bucket = TypeAdapter(dict[str, JsonValue]).validate_python(request_data["metadata"])
    records = TypeAdapter(list[GuardrailRecord]).validate_python(
        bucket["standard_logging_guardrail_information"]
    )
    return [record["guardrail_status"] for record in records]


def attack_names(message: str) -> list[str]:
    return [
        line.removeprefix("- ").split(":", 1)[0].lower()
        for line in message.splitlines()
        if line.startswith("- ") and ":" in line
    ]


def attack_details(body: JsonValue, attacks: list[str]) -> dict[str, JsonValue]:
    response = TypeAdapter(ResponseView).validate_python(body)
    details = response.get("details")
    if details is None:
        return {}
    return {
        attack: details[attack] if attack in details else dict[str, JsonValue]()
        for attack in attacks
    }


async def detect(call: DetectCall) -> DetectOutput:
    guard = build(call["settings"])
    transport = RecordingTransport(call["answer"])
    guard.async_handler.client = httpx.AsyncClient(
        transport=httpx.MockTransport(transport)
    )
    send = cast(Apply, guard.apply_guardrail)  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/enkryptai/enkryptai.py#L455-L499
    request_data: dict[str, JsonValue] = {"model": "enkryptai"}
    raised = False
    attacks: list[str] = []
    try:
        inputs: dict[str, list[str]] = {"texts": call["texts"]}
        await send(inputs, request_data, "request", None)
    except ValueError as error:
        raised = True
        attacks = attack_names(str(error))
    except httpx.HTTPError:
        raised = True
    details = attack_details(call["answer"]["body"], attacks)
    result: ResultView | None = None if raised else {"texts": call["texts"]}
    return {
        "requests": [view(request) for request in transport.requests],
        "raised": raised,
        "statuses": statuses(request_data),
        "processed": {"attacks": attacks, "details": details},
        "result": result,
    }


def run(input_value: dict[str, JsonValue]) -> ConstructOutput | DetectOutput:
    if "construct" in input_value:
        requested = TypeAdapter(ConstructCall).validate_python(input_value["construct"])
        return construct(requested["settings"])
    if "detect" in input_value:
        return asyncio.run(detect(TypeAdapter(DetectCall).validate_python(input_value["detect"])))
    raise ValueError("unsupported operation")
