from typing import TypeAlias

import httpx
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

from azure_embeddings.types import ClientName, Step

BODY = TypeAdapter(dict[str, Json])
HEADER_VALUES = TypeAdapter(dict[str, str])
ENDPOINT = "https://fixture.openai.azure.com"
V1_URL = "https://fixture.openai.azure.com/openai/v1/"
API_VERSION = "2024-02-01"
CLIENT_KEY = "fixture-key"

SdkClient: TypeAlias = AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI


def header_names(step: Step) -> list[str]:
    names: list[str] = []
    if "headers" in step:
        names.extend(name.lower() for name in step["headers"])
    if "optional_params" in step and "extra_headers" in step["optional_params"]:
        extra = HEADER_VALUES.validate_python(step["optional_params"]["extra_headers"])
        names.extend(name.lower() for name in extra)
    return names


class Session:
    def __init__(self, step: Step) -> None:
        self.answer = step["answer"]
        self.names = header_names(step)
        # the SDK adds encoding_format=base64 when the caller sent none
        self.sdk_adds_format = (
            "optional_params" not in step or "encoding_format" not in step["optional_params"]
        )
        self.calls: list[Json] = []

    def call_view(self, request: httpx.Request) -> dict[str, Json]:
        body = BODY.validate_json(request.content)
        if self.sdk_adds_format:
            body.pop("encoding_format", None)
        headers: dict[str, Json] = {name: request.headers.get(name) for name in self.names}
        timeout: float | None = None
        match request.extensions:
            case {"timeout": {"read": float() as read}}:
                timeout = read
        return {"body": body, "headers": headers, "timeout": timeout}

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.calls.append(self.call_view(request))
        answer = self.answer
        if "connection" in answer:
            raise httpx.ConnectError("fixture connection failure", request=request)
        reply = answer["reply"]
        return httpx.Response(
            reply["status"],
            headers=reply["headers"],
            content=reply["body"].encode(),
            request=request,
        )


def make_client(name: ClientName, session: Session) -> SdkClient:
    match name:
        case "azure_open_ai":
            return AzureOpenAI(
                api_key=CLIENT_KEY,
                azure_endpoint=ENDPOINT,
                api_version=API_VERSION,
                max_retries=0,
                http_client=httpx.Client(transport=httpx.MockTransport(session.handle)),
            )
        case "async_azure_open_ai":
            return AsyncAzureOpenAI(
                api_key=CLIENT_KEY,
                azure_endpoint=ENDPOINT,
                api_version=API_VERSION,
                max_retries=0,
                http_client=httpx.AsyncClient(transport=httpx.MockTransport(session.handle)),
            )
        case "open_ai":
            return OpenAI(
                api_key=CLIENT_KEY,
                base_url=V1_URL,
                max_retries=0,
                http_client=httpx.Client(transport=httpx.MockTransport(session.handle)),
            )
        case "async_open_ai":
            return AsyncOpenAI(
                api_key=CLIENT_KEY,
                base_url=V1_URL,
                max_retries=0,
                http_client=httpx.AsyncClient(transport=httpx.MockTransport(session.handle)),
            )
