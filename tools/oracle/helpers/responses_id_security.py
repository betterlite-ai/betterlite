import asyncio
import json
import os
import sys
from collections.abc import AsyncGenerator, Generator
from contextlib import contextmanager
from types import SimpleNamespace
from typing import Final, Generic, Literal, TypedDict, TypeVar

from fastapi import HTTPException
from litellm.caching.dual_cache import DualCache
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.common_utils.encrypt_decrypt_utils import decrypt_value_helper
from litellm.proxy.hooks.responses_id_security import ResponsesIDSecurity
from litellm.types.llms import openai
from litellm.types.llms.base import BaseLiteLLMOpenAIResponseObject as EventBase
from litellm.types.utils import ADDRESSED_RESPONSE_ID_FIELD, CallTypesLiteral
from pydantic import JsonValue, TypeAdapter, ValidationError

MAPPING: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
TEXT: TypeAdapter[str] = TypeAdapter(str)
SALT_ENV: Final = "LITELLM_SALT_KEY"
PROXY_SERVER_NAME: Final = "litellm.proxy.proxy_server"
ReturnValue = TypeVar("ReturnValue")


class SettingsInput(TypedDict):
    disable: bool
    allow: bool
    signing_key: str | None


class PrincipalInput(TypedDict):
    user_id: str | None
    team_id: str | None
    role: Literal["PROXY_ADMIN", "PROXY_ADMIN_VIEW_ONLY"] | None
    role_form: Literal["enum", "value"] | None


class CallInput(TypedDict):
    operation: Literal["authorize", "success", "stream"]
    settings: SettingsInput
    principal: PrincipalInput
    call_type: CallTypesLiteral
    data: dict[str, JsonValue]
    response: dict[str, JsonValue]
    route: str | None
    events: list[dict[str, JsonValue]]


class Reader(Generic[ReturnValue]):
    def __init__(self, value: ReturnValue) -> None:
        self.value = value

    def __call__(self) -> ReturnValue:
        return self.value


@contextmanager
def proxy_salt(signing_key: str | None) -> Generator[None]:
    saved = os.environ.pop(SALT_ENV, None)
    if signing_key is not None:
        os.environ[SALT_ENV] = signing_key
    saved_module = sys.modules.get(PROXY_SERVER_NAME)
    sys.modules[PROXY_SERVER_NAME] = SimpleNamespace(master_key=None, general_settings={})  # type: ignore[assignment, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/responses_id_security.py#L49-L55
    try:
        yield
    finally:
        sys.modules.pop(PROXY_SERVER_NAME, None)
        if saved_module is not None:
            sys.modules[PROXY_SERVER_NAME] = saved_module
        os.environ.pop(SALT_ENV, None)
        if saved is not None:
            os.environ[SALT_ENV] = saved


def response_hook(settings: SettingsInput) -> ResponsesIDSecurity:
    general: dict[str, bool] = {
        "disable_responses_id_security": settings["disable"],
        "allow_unmanaged_response_ids": settings["allow"],
    }
    signing_key = settings["signing_key"]
    return ResponsesIDSecurity(
        general_settings_reader=Reader(general),
        signing_key_reader=Reader(signing_key),
    )


def principal_user(principal: PrincipalInput, route: str | None) -> UserAPIKeyAuth:
    user_id, team_id, role = principal["user_id"], principal["team_id"], principal["role"]
    if role is None:
        return UserAPIKeyAuth(user_id=user_id, team_id=team_id, request_route=route)
    raw_role = "proxy_admin" if role == "PROXY_ADMIN" else "proxy_admin_viewer"
    user_role = LitellmUserRoles(raw_role)
    return UserAPIKeyAuth(
        user_id=user_id, team_id=team_id, user_role=user_role, request_route=route
    )


@contextmanager
def hook_session(
    settings: SettingsInput, principal: PrincipalInput, route: str | None
) -> Generator[tuple[ResponsesIDSecurity, UserAPIKeyAuth]]:
    with proxy_salt(settings["signing_key"]):
        yield response_hook(settings), principal_user(principal, route)


def text_or_none(value: JsonValue) -> str | None:
    try:
        return TEXT.validate_python(value, strict=True)
    except ValidationError:
        return None


def nested_mapping(value: JsonValue) -> dict[str, JsonValue] | None:
    try:
        return MAPPING.validate_python(value, strict=True)
    except ValidationError:
        return None


def project(value: JsonValue) -> str:
    text = text_or_none(value)
    if text is None:
        return f"json:{json.dumps(value, sort_keys=True, separators=(',', ':'))}"
    if "resp_" not in text:
        return f"plain:{text}"
    segment = text.split("resp_")[1]
    plaintext = decrypt_value_helper(value=segment, key="response_id", return_original_value=True)
    if plaintext is None or plaintext == segment:
        projected = f"plain:{text}"
    elif not plaintext.startswith("litellm_proxy"):
        projected = f"decrypted:{plaintext}"
    else:
        parts = plaintext.split(";")
        if not parts[1:]:
            projected = "fallback"
        elif not parts[2:]:
            projected = "malformed"
        else:
            original = parts[0].split("response_id:")[-1]
            user = parts[1].split("user_id:")[-1]
            team = parts[2].split("team_id:")[-1]
            projected = f"managed:{original}|{user}|{team}"
    return projected


def project_nested(value: JsonValue) -> str:
    mapping = nested_mapping(value)
    if mapping is None or "id" not in mapping:
        return project(value)
    return project(mapping["id"])


def advertised_id(dumped: dict[str, JsonValue]) -> str | None:
    top = text_or_none(dumped.get("id"))
    if top is not None:
        return top
    mapping = nested_mapping(dumped.get("response"))
    if mapping is None:
        return None
    return text_or_none(mapping.get("id"))


def parse_event(raw: dict[str, JsonValue]) -> EventBase:
    if raw.get("type") == "response.created":
        return openai.ResponseCreatedEvent.model_validate(raw)
    if raw.get("type") == "response.completed":
        return openai.ResponseCompletedEvent.model_validate(raw)
    return EventBase.model_validate(raw)


async def chunk_source(chunks: list[EventBase]) -> AsyncGenerator[EventBase, None]:
    for chunk in chunks:
        yield chunk


async def stream_run(
    hook: ResponsesIDSecurity,
    user: UserAPIKeyAuth,
    events: list[dict[str, JsonValue]],
) -> tuple[list[JsonValue], list[str], list[str | None]]:
    chunks = [parse_event(event) for event in events]
    request_data: dict[str, JsonValue] = {}
    stream = hook.async_post_call_streaming_iterator_hook(user, chunk_source(chunks), request_data)  # pyright: ignore[reportAny, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/responses_id_security.py#L328
    produced = [chunk async for chunk in stream]
    views: list[JsonValue] = []
    raw: list[str] = []
    advertised: list[str | None] = []
    for chunk in produced:
        text = chunk.model_dump_json()
        dumped = MAPPING.validate_json(text)
        nested = project_nested(dumped["response"]) if "response" in dumped else None
        top = project(dumped["id"]) if "id" in dumped else None
        views.append({"top": top, "nested": nested})
        raw.append(text)
        advertised.append(advertised_id(dumped))
    return views, raw, advertised


def addressed_field(call_type: str) -> str | None:
    if call_type == "aresponses":
        return "previous_response_id"
    reads = {"aget_responses", "adelete_responses", "acancel_responses", "alist_input_items"}
    return "response_id" if call_type in reads else None


def project_field(data: dict[str, JsonValue], field_name: str | None) -> str | None:
    if field_name is None or field_name not in data:
        return None
    return project(data[field_name])


def forbidden_kind(detail: str) -> str:
    if detail.startswith("Forbidden. This response id was not issued"):
        return "unmanaged_forbidden"
    if detail.startswith("Forbidden. The response id is not associated with the user"):
        return "user_forbidden"
    return "team_forbidden"


def call_answer(
    kind: str,
    detail: str | None,
    status: int | None,
    class_name: str | None,
    scope: tuple[dict[str, JsonValue], str | None],
) -> JsonValue:
    data, field_name = scope
    field = project_field(data, field_name)
    retained = project_field(data, ADDRESSED_RESPONSE_ID_FIELD)
    call: dict[str, JsonValue] = {
        "field": field, "retained": retained, "kind": kind, "detail": detail, "type": None,
        "param": None, "code": None, "status": status, "class": class_name,
    }
    return {"call": call}


def authorize_outcome(
    data: dict[str, JsonValue],
    call_type: CallTypesLiteral,
    hook: ResponsesIDSecurity,
    user: UserAPIKeyAuth,
) -> tuple[str, str | None, int | None, str | None]:
    cache = DualCache()
    try:
        no_result = asyncio.run(hook.async_pre_call_hook(user, cache, data, call_type)) is None  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/responses_id_security.py#L116
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/responses_id_security.py#L171
        detail = error.detail
        return forbidden_kind(detail), detail, error.status_code, None
    except IndexError:
        return "internal_error", None, 500, "IndexError"
    if no_result:
        return "not_responses_call", None, None, None
    kind = "authorized" if text_or_none(data.get(ADDRESSED_RESPONSE_ID_FIELD)) else "unchanged"
    return kind, None, None, None


def authorize_answer(case: CallInput) -> JsonValue:
    data = case["data"]
    field_name = addressed_field(case["call_type"])
    with hook_session(case["settings"], case["principal"], None) as (hook, user):
        kind, detail, status, class_name = authorize_outcome(
            data, case["call_type"], hook, user
        )
        return call_answer(kind, detail, status, class_name, (data, field_name))


def success_answer(case: CallInput) -> JsonValue:
    with hook_session(case["settings"], case["principal"], case["route"]) as (hook, user):
        response = openai.ResponsesAPIResponse.model_validate(case["response"])  # type: ignore[misc]  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1388
        request_data: dict[str, JsonValue] = {}
        asyncio.run(hook.async_post_call_success_hook(request_data, user, response))  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/responses_id_security.py#L305
        dumped = MAPPING.validate_json(response.model_dump_json())
        nested = project_nested(dumped["response"]) if "response" in dumped else None
        return {"response": {"id": project(dumped["id"]), "nested": nested}}


def stream_answer(case: CallInput) -> JsonValue:
    with hook_session(case["settings"], case["principal"], case["route"]) as (hook, user):
        first = asyncio.run(stream_run(hook, user, case["events"]))
        second = asyncio.run(stream_run(hook, user, case["events"]))
    head = first[2][0] if first[2] else None
    shared = head is not None and all(value == head for value in first[2])
    return {
        "stream": {
            "events": first[0],
            "shared": shared,
            "second_request_differs": first[1] != second[1],
        },
    }


def run(input_value: CallInput) -> JsonValue:
    if input_value["operation"] == "authorize":
        return authorize_answer(input_value)
    if input_value["operation"] == "success":
        return success_answer(input_value)
    return stream_answer(input_value)
