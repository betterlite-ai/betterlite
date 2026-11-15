import asyncio
from typing import Literal, Protocol, TypedDict, cast

import httpx
from litellm.proxy.guardrails.guardrail_hooks.qohash.qohash import QostodianNexus
from pydantic import JsonValue, TypeAdapter


class GuardrailBody(TypedDict, total=False):
    input_type: str
    litellm_call_id: str | None
    litellm_trace_id: str | None
    texts: list[str] | None
    images: list[str] | None
    request_headers: dict[str, str] | None


class Answer(TypedDict):
    status: int
    body: JsonValue


class Settings(TypedDict):
    guardrail_name: str
    extra_headers: list[str]


class Inputs(TypedDict, total=False):
    texts: list[str]


class Scanned(TypedDict, total=False):
    texts: list[str]


class Input(TypedDict, total=False):
    settings: Settings
    inputs: Inputs
    request_data: dict[str, JsonValue]
    input_type: Literal["request", "response"]
    answer: Answer


class Output(TypedDict):
    guardrail_name: str
    path: str
    content_type: str | None
    request: GuardrailBody
    inputs: dict[str, list[str] | None]


class Nexus(Protocol):
    async def apply_guardrail(
        self,
        inputs: Inputs,
        request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"],
    ) -> Scanned: ...


BODY_ADAPTER: TypeAdapter[GuardrailBody] = TypeAdapter(GuardrailBody)


class Recorder:
    def __init__(self, answer: Answer | None) -> None:
        self.answer = answer
        self.body: GuardrailBody | None = None
        self.path = ""
        self.content_type: str | None = None

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.path = request.url.path
        self.content_type = request.headers.get("content-type")
        self.body = BODY_ADAPTER.validate_json(request.content)
        if self.answer is None:
            raise httpx.ReadTimeout("the mock never answers", request=request)
        return httpx.Response(self.answer["status"], json=self.answer["body"])


def run(input_value: Input) -> Output:
    settings = input_value.get("settings")
    if settings is None:
        settings = Settings(guardrail_name="", extra_headers=[])
    inputs = input_value.get("inputs")
    if inputs is None:
        inputs = Inputs()
    request_data = input_value.get("request_data")
    if request_data is None:
        request_data = {}
    input_type = input_value.get("input_type")
    if input_type is None:
        input_type = "request"
    texts = inputs.get("texts")
    if texts is None:
        texts = []
    recorder = Recorder(input_value.get("answer"))
    real = QostodianNexus(
        api_base="http://oracle.invalid",
        guardrail_name=settings["guardrail_name"],
        extra_headers=settings["extra_headers"],
    )
    real.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(recorder))
    guardrail = cast(Nexus, real)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/qohash/qohash.py#L49-L70
    result = asyncio.run(
        guardrail.apply_guardrail(
            inputs=Inputs(texts=texts), request_data=request_data, input_type=input_type
        )
    )
    body = recorder.body
    if body is None:
        raise ValueError("the guardrail sent no request")
    return {
        "guardrail_name": settings["guardrail_name"],
        "path": recorder.path,
        "content_type": recorder.content_type,
        "request": body,
        "inputs": {"texts": result.get("texts")},
    }
