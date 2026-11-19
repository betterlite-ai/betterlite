from urllib.parse import parse_qsl

import httpx
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue

from .types import (
    API_BASE,
    API_VERSION,
    JSON,
    OPTIONAL_HEADER_MAP,
    STATUS,
    TEXT,
    ErrorSpec,
    ExchangeSpec,
    SendSpec,
    StreamCase,
)


class ScriptedError(Exception):
    status_code: int | None
    message: str | None
    headers: httpx.Headers | dict[str, str] | None
    body: JsonValue
    response: httpx.Response | None

    def __init__(self, spec: ErrorSpec) -> None:
        super().__init__(spec["text"])
        status = spec.get("status")
        if status is not None and status["present"]:
            self.status_code = STATUS.validate_python(status.get("value"))
        message = spec.get("message")
        if message is not None and message["present"]:
            self.message = TEXT.validate_python(message.get("value"))
        headers = spec.get("headers")
        if headers is not None and headers["present"]:
            self.headers = OPTIONAL_HEADER_MAP.validate_python(headers.get("value"))
        body = spec.get("body")
        if body is not None and body["present"]:
            self.body = JSON.validate_python(body.get("value"))
        response = spec.get("response")
        if response is not None and response["present"]:
            response_headers = OPTIONAL_HEADER_MAP.validate_python(response.get("value"))
            if response_headers is None:
                self.response = None
            else:
                self.response = httpx.Response(
                    200,
                    headers=response_headers,
                    request=httpx.Request("POST", API_BASE),
                )


class Fixture:
    def __init__(self, send: SendSpec) -> None:
        self.send = send
        self.requests: list[JsonValue] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append({"data": JSON.validate_json(request.content)})
        return self.response(request)

    def response(self, request: httpx.Request) -> httpx.Response:
        status = self.send.get("status")
        if status is None:
            status = 200
        headers = self.send.get("headers")
        if "headers" not in self.send:
            headers = {"content-type": "application/json"}
        events = self.send.get("events")
        if events is not None:
            return httpx.Response(
                status, headers=headers, content="".join(events).encode(), request=request
            )
        body = self.send.get("body")
        if body is None:
            body = ""
        return httpx.Response(status, headers=headers, content=body.encode(), request=request)


class TokenEndpoint:
    def __init__(self, spec: ExchangeSpec) -> None:
        self.spec = spec
        self.forms: list[JsonValue] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        form = dict(parse_qsl(request.content.decode()))
        self.forms.append({"url": str(request.url), "form": form})
        status = self.spec.get("status")
        if status is None:
            status = 200
        body = self.spec.get("body")
        if body is None:
            body = ""
        return httpx.Response(status, content=body.encode(), request=request)


def scripted_provider() -> str:
    return "provider-token"


def client_for(
    case: StreamCase, fixture: Fixture
) -> AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI | None:
    supplied = case.get("supplied")
    if supplied is None:
        return None
    kind = supplied.get("kind")
    if kind is None:
        kind = "azure_sync"
    key = supplied.get("api_key")
    version = case.get("api_version")
    if version is None:
        version = API_VERSION
    base = case.get("api_base")
    if base is None:
        base = API_BASE
    transport = httpx.MockTransport(fixture)
    if kind == "azure_sync":
        return AzureOpenAI(
            api_key=key,
            azure_endpoint=base,
            api_version=version,
            max_retries=0,
            http_client=httpx.Client(transport=transport),
        )
    if kind == "azure_async":
        return AsyncAzureOpenAI(
            api_key=key,
            azure_endpoint=base,
            api_version=version,
            max_retries=0,
            http_client=httpx.AsyncClient(transport=transport),
        )
    if kind == "open_ai_sync":
        return OpenAI(
            api_key=key,
            base_url=base,
            max_retries=0,
            http_client=httpx.Client(transport=transport),
        )
    return AsyncOpenAI(
        api_key=key,
        base_url=base,
        max_retries=0,
        http_client=httpx.AsyncClient(transport=transport),
    )
