import asyncio
from typing import Literal, Protocol, TypeAlias, TypedDict

import httpx
from fastapi import HTTPException
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.pillar.pillar import PillarGuardrail
from pydantic import JsonValue, TypeAdapter

API_BASE = "https://pillar.test"
API_KEY = "test-key"
OBJECT = TypeAdapter(dict[str, JsonValue])
JSON = TypeAdapter(JsonValue)
Setting: TypeAlias = Literal[
    "async_mode", "persist_session", "include_scanners", "include_evidence"
]


class Reply(TypedDict):
    status: int
    body: JsonValue


class Settings(TypedDict):
    api_key: str | None
    on_flagged_action: str | None
    async_mode: bool | None
    persist_session: bool | None
    include_scanners: bool | None
    include_evidence: bool | None
    fallback_on_error: str | None


class Key(TypedDict):
    key_name: str | None
    key_alias: str | None
    user_id: str | None
    user_email: str | None
    team_id: str | None
    team_alias: str | None
    org_id: str | None


class Captured(TypedDict):
    method: str
    path: str
    headers: dict[str, str]
    body: JsonValue


class ResolveBoolInput(TypedDict):
    function: Literal["resolve_bool"]
    setting: Setting
    provided: bool | None
    environment: dict[str, str]


class ProtectInput(TypedDict):
    function: Literal["protect"]
    settings: Settings
    key: Key
    data: JsonValue
    answer: Reply


Input: TypeAlias = ResolveBoolInput | ProtectInput


class Runner(Protocol):
    async def run_pillar_guardrail(
        self, data: dict[str, JsonValue], user_api_key_dict: UserAPIKeyAuth
    ) -> dict[str, JsonValue]: ...


def reported_header(name: str) -> bool:
    lowered = name.lower()
    return (
        lowered == "content-type"
        or lowered.startswith("plr_")
        or lowered.startswith("x-litellm-")
    )


class Scripted:
    def __init__(self, reply: Reply) -> None:
        self.reply = reply
        self.calls: list[Captured] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        captured = Captured(
            method=request.method,
            path=request.url.path,
            headers={
                name.lower(): value
                for name, value in request.headers.items()
                if reported_header(name)
            },
            body=JSON.validate_json(request.content),
        )
        self.calls.append(captured)
        return httpx.Response(self.reply["status"], json=self.reply["body"])


def build_runner(settings: Settings, scripted: Scripted) -> Runner:
    guard = PillarGuardrail(
        api_key=settings["api_key"],
        api_base=API_BASE,
        on_flagged_action=settings["on_flagged_action"],
        async_mode=settings["async_mode"],
        persist_session=settings["persist_session"],
        include_scanners=settings["include_scanners"],
        include_evidence=settings["include_evidence"],
        fallback_on_error=settings["fallback_on_error"],
    )
    guard.async_handler = AsyncHTTPHandler(transport=httpx.MockTransport(scripted))
    return guard


def key_context(key: Key) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        key_name=key["key_name"],
        key_alias=key["key_alias"],
        user_id=key["user_id"],
        user_email=key["user_email"],
        team_id=key["team_id"],
        team_alias=key["team_alias"],
        org_id=key["org_id"],
    )


def protect(settings: Settings, key: Key, request_data: JsonValue, answer: Reply) -> JsonValue:
    scripted = Scripted(answer)
    runner = build_runner(settings, scripted)
    data = OBJECT.validate_python(request_data)
    status = 200
    try:
        asyncio.run(runner.run_pillar_guardrail(data, key_context(key)))
    except HTTPException as error:
        status = error.status_code
    request: JsonValue = OBJECT.validate_python(scripted.calls[0]) if scripted.calls else None
    return JSON.validate_python({"request": request, "data": data, "status": status})


def resolve_bool(setting: Setting, *, provided: bool | None) -> JsonValue:
    value: bool | None = None
    match setting:
        case "async_mode":
            value = PillarGuardrail(
                api_key=API_KEY, api_base=API_BASE, async_mode=provided
            ).async_mode
        case "persist_session":
            value = PillarGuardrail(
                api_key=API_KEY, api_base=API_BASE, persist_session=provided
            ).persist_session
        case "include_scanners":
            value = PillarGuardrail(
                api_key=API_KEY, api_base=API_BASE, include_scanners=provided
            ).include_scanners
        case "include_evidence":
            value = PillarGuardrail(
                api_key=API_KEY, api_base=API_BASE, include_evidence=provided
            ).include_evidence
    return JSON.validate_python({"value": value})


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "resolve_bool":
        return resolve_bool(input_value["setting"], provided=input_value["provided"])
    return protect(
        input_value["settings"], input_value["key"], input_value["data"], input_value["answer"]
    )
