from typing import TypeAlias

import httpx
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

from azure_embeddings.types import ClientName, Step

BODY = TypeAdapter(dict[str, Json])
ENDPOINT = "https://fixture.openai.azure.com"
V1_URL = "https://fixture.openai.azure.com/openai/v1/"
API_VERSION = "2024-02-01"
CLIENT_KEY = "fixture-key"

SdkClient: TypeAlias = AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI


class Session:
    def __init__(self, step: Step) -> None:
        self.answer = step["answer"]
        # the SDK adds encoding_format=base64 when the caller sent none
        self.sdk_adds_format = (
            "optional_params" not in step or "encoding_format" not in step["optional_params"]
        )
        self.calls: list[Json] = []

    def call_view(self, request: httpx.Request) -> dict[str, Json]:
        body = BODY.validate_json(request.content)
        if self.sdk_adds_format:
            body.pop("encoding_format", None)
        timeout: float | None = None
        match request.extensions:
            case {"timeout": {"read": float() as read}}:
                timeout = read
        return {"body": body, "timeout": timeout}

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
    transport = httpx.MockTransport(session.handle)
    match name:
        case "azure_open_ai":
            return AzureOpenAI(
                api_key=CLIENT_KEY,
                azure_endpoint=ENDPOINT,
                api_version=API_VERSION,
                max_retries=0,
                http_client=httpx.Client(transport=transport),
            )
        case "async_azure_open_ai":
            return AsyncAzureOpenAI(
                api_key=CLIENT_KEY,
                azure_endpoint=ENDPOINT,
                api_version=API_VERSION,
                max_retries=0,
                http_client=httpx.AsyncClient(transport=transport),
            )
        case "open_ai":
            return OpenAI(
                api_key=CLIENT_KEY,
                base_url=V1_URL,
                max_retries=0,
                http_client=httpx.Client(transport=transport),
            )
        case "async_open_ai":
            return AsyncOpenAI(
                api_key=CLIENT_KEY,
                base_url=V1_URL,
                max_retries=0,
                http_client=httpx.AsyncClient(transport=transport),
            )
