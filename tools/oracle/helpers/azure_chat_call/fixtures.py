import asyncio
from typing import Literal

import httpx
from openai import AsyncAzureOpenAI, AzureOpenAI
from pydantic import JsonValue

from .types import (
    API_BASE,
    API_VERSION,
    BODY,
    CHAT_BODY,
    JSON,
    TIMEOUT,
    Answer,
    AzureClient,
    CallCase,
)


def api_base_of(case: CallCase) -> str:
    base = case.get("api_base")
    if base is None:
        return API_BASE
    return base


def api_version_of(case: CallCase) -> str:
    version = case.get("api_version")
    if version is None:
        return API_VERSION
    return version


def timeout_of(case: CallCase) -> float:
    timeout = case.get("timeout")
    if timeout is None:
        return TIMEOUT
    return timeout


def case_answer(case: CallCase) -> Answer:
    answer = case.get("answer")
    if answer is None:
        return {}
    return answer


def optional_params_of(case: CallCase) -> dict[str, JsonValue]:
    params = case.get("optional_params")
    if params is None:
        return {}
    return dict(params)


def litellm_params_of(case: CallCase) -> dict[str, JsonValue]:
    params = case.get("litellm_params")
    merged: dict[str, JsonValue] = {} if params is None else dict(params)
    base = case.get("base_model")
    if base is not None:
        merged["base_model"] = base
    return merged


def caller_of(case: CallCase) -> JsonValue:
    headers = case.get("headers")
    if headers is None:
        return None
    return JSON.validate_python(headers)


def source_key(case: CallCase) -> str | None:
    if "client_api_key" in case:
        return case["client_api_key"]
    return case.get("api_key")


def client_kind(case: CallCase) -> Literal["sync", "async"]:
    requested = case.get("client")
    if requested is not None:
        return requested
    if case.get("acompletion"):
        return "async"
    return "sync"


def client_for(case: CallCase, fixture: "Fixture") -> AzureClient:
    key = source_key(case)
    token = None if key is not None else case.get("azure_ad_token")
    version = api_version_of(case)
    endpoint = api_base_of(case)
    if client_kind(case) == "async":
        return AsyncAzureOpenAI(
            api_key=key,
            azure_ad_token=token,
            api_version=version,
            azure_endpoint=endpoint,
            max_retries=0,
            http_client=httpx.AsyncClient(transport=httpx.MockTransport(fixture.respond)),
        )
    return AzureOpenAI(
        api_key=key,
        azure_ad_token=token,
        api_version=version,
        azure_endpoint=endpoint,
        max_retries=0,
        http_client=httpx.Client(transport=httpx.MockTransport(fixture)),
    )


def supplied_client(case: CallCase, mock: AzureClient) -> AzureClient | None:
    if case.get("supplied") is not False:
        return mock
    return None


def response_for(answer: Answer, request: httpx.Request) -> httpx.Response:
    error = answer.get("error")
    if error == "timeout":
        raise httpx.ReadTimeout("Request timed out.", request=request)
    if error == "other":
        raise httpx.ConnectError("connection refused", request=request)
    status = answer.get("status")
    if status is None:
        status = 400 if error == "bad_request" else 200
    headers = answer.get("headers")
    if headers is None:
        headers = {"content-type": "application/json"}
    body = answer.get("body")
    if body is None:
        body = CHAT_BODY
    return httpx.Response(status, headers=headers, content=body.encode(), request=request)


class Fixture:
    def __init__(self, answer: Answer, caller: JsonValue) -> None:
        self.answer = answer
        self.caller = caller
        self.requests: list[JsonValue] = []
        self.started = asyncio.Event()

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(self.view(request))
        return response_for(self.answer, request)

    async def respond(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(self.view(request))
        self.started.set()
        if self.answer.get("error") == "pending":
            await asyncio.Event().wait()
        return response_for(self.answer, request)

    def view(self, request: httpx.Request) -> JsonValue:
        body = BODY.validate_json(request.content)
        body.pop("extra_headers", None)
        return {"data": body, "caller_headers": self.caller}
