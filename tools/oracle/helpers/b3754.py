import asyncio
import copy
import os
import ssl
from typing import Literal, TypeAlias, TypedDict

import httpx
from fastapi import HTTPException
from litellm.proxy.guardrails.guardrail_hooks.cato_networks.cato_networks import (
    CatoNetworksGuardrail,
)
from pydantic import JsonValue, TypeAdapter

BODY: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class Settings(TypedDict, total=False):
    api_key: str
    api_base: str
    inspect_embeddings: bool
    ssl_verify: bool | str


class Answer(TypedDict):
    status: int
    body: JsonValue


class ConstructInput(TypedDict):
    function: Literal["construct"]
    settings: Settings
    environment: dict[str, str]


class AnalyzeInput(TypedDict):
    function: Literal["analyze"]
    settings: Settings
    environment: dict[str, str]
    data: dict[str, JsonValue]
    hook: Literal["pre_call", "moderation", "output"]
    key_alias: str | None
    user_email: str | None
    answer: Answer | None


Input: TypeAlias = ConstructInput | AnalyzeInput


class Recorder:
    def __init__(self, answer: Answer | None) -> None:
        self.answer = answer
        self.seen: list[httpx.Request] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(request)
        if self.answer is None:
            raise httpx.ReadTimeout("the mock never answers", request=request)
        return httpx.Response(self.answer["status"], json=self.answer["body"])


class WsSslProbe(CatoNetworksGuardrail):
    def ws_ssl_kind(self) -> JsonValue:
        value = self._ws_connect_ssl_kwargs.get("ssl")
        if value is None:
            return None
        match value:
            case ssl.SSLContext():
                return "disabled" if value.verify_mode == ssl.CERT_NONE else "system_roots"
            case bool():
                return "system_roots" if value else "disabled"
            case str():
                return {"ca_file": value}


def build(settings: Settings, environment: dict[str, str]) -> WsSslProbe:
    os.environ.update(environment)
    return WsSslProbe(
        guardrail_name="cato",
        api_key=settings.get("api_key"),
        api_base=settings.get("api_base"),
        inspect_embeddings=settings.get("inspect_embeddings"),
        ssl_verify=settings.get("ssl_verify"),
    )


def construct_answer(input_value: ConstructInput) -> dict[str, JsonValue]:
    guardrail = build(input_value["settings"], input_value["environment"])
    supported = guardrail.supported_event_hooks
    hooks: list[JsonValue] = [hook.value for hook in supported] if supported is not None else []
    answer: dict[str, JsonValue] = {
        "api_key": guardrail.api_key,
        "api_base": guardrail.api_base,
        "ws_api_base": guardrail.ws_api_base,
        "inspect_embeddings": guardrail.inspect_embeddings,
        "supported_event_hooks": hooks,
        "ws_ssl": guardrail.ws_ssl_kind(),
    }
    return answer


async def run_guardrail(
    guardrail: CatoNetworksGuardrail,
    data: dict[str, JsonValue],
    hook: str,
    key_alias: str | None,
    user_email: str | None,
) -> None:
    await guardrail.call_cato_guardrail(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/cato_networks/cato_networks.py#L295-L300
        data, hook=hook, key_alias=key_alias, user_email=user_email
    )


def summarize(
    guardrail: CatoNetworksGuardrail,
    data: dict[str, JsonValue],
    seen: list[httpx.Request],
    rejected: JsonValue,
) -> dict[str, JsonValue]:
    answer: dict[str, JsonValue] = {
        "path": None,
        "headers": None,
        "body": None,
        "data": data,
        "rejected": rejected,
    }
    if seen:
        request = seen[0]
        answer["path"] = request.url.path
        answer["headers"] = {
            "authorization_bearer_ok": request.headers.get("authorization")
            == f"Bearer {guardrail.api_key}",
            "x-cato-litellm-hook": request.headers.get("x-cato-litellm-hook"),
            "x-cato-litellm-version": request.headers.get("x-cato-litellm-version"),
            "x-cato-call-id": request.headers.get("x-cato-call-id"),
            "x-cato-user-email": request.headers.get("x-cato-user-email"),
            "x-cato-gateway-key-alias": request.headers.get("x-cato-gateway-key-alias"),
            "content_type": request.headers.get("content-type"),
        }
        answer["body"] = BODY.validate_json(request.content)
    return answer


def analyze_answer(input_value: AnalyzeInput) -> dict[str, JsonValue]:
    guardrail = build(input_value["settings"], input_value["environment"])
    recorder = Recorder(input_value["answer"])
    guardrail.async_handler.client = httpx.AsyncClient(
        transport=httpx.MockTransport(recorder.handle)
    )
    data = copy.deepcopy(input_value["data"])
    rejected: JsonValue = None
    try:
        asyncio.run(
            run_guardrail(
                guardrail,
                data,
                input_value["hook"],
                input_value["key_alias"],
                input_value["user_email"],
            )
        )
    except HTTPException as error:
        status: dict[str, JsonValue] = {"status": error.status_code}
        rejected = status
    return summarize(guardrail, data, recorder.seen, rejected)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "construct":
        return construct_answer(input_value)
    return analyze_answer(input_value)
