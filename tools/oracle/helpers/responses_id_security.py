import asyncio
import json
import os
import sys
from collections.abc import AsyncGenerator, Callable, Generator, Mapping
from contextlib import contextmanager
from types import ModuleType
from typing import Final, Literal, NamedTuple, TypedDict, cast

from fastapi import HTTPException
from litellm.caching.dual_cache import DualCache
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.common_utils.encrypt_decrypt_utils import (
    decrypt_value_helper,
    encrypt_value_helper,
)
from litellm.proxy.hooks.responses_id_security import ResponsesIDSecurity
from litellm.types.llms.base import BaseLiteLLMOpenAIResponseObject as EventBase
from litellm.types.llms.openai import ResponseCreatedEvent, ResponsesAPIResponse
from litellm.types.utils import ADDRESSED_RESPONSE_ID_FIELD, CallTypesLiteral, SpecialEnums
from pydantic import JsonValue, TypeAdapter, ValidationError

TEXT: TypeAdapter[str] = TypeAdapter(str)
MAPPING: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
MANAGED_TEMPLATE: Final = SpecialEnums.LITELLM_MANAGED_RESPONSE_API_RESPONSE_ID_COMPLETE_STR.value
SALT_ENV: Final = "LITELLM_SALT_KEY"
PROXY_SERVER_NAME: Final = "litellm.proxy.proxy_server"
# the wire role names the cases use, and the members the hook compares against
ROLE_MEMBERS: Final[Mapping[str, LitellmUserRoles]] = {
    "PROXY_ADMIN": LitellmUserRoles.PROXY_ADMIN,
    "PROXY_ADMIN_VIEW_ONLY": LitellmUserRoles.PROXY_ADMIN_VIEW_ONLY,
}


class SettingsInput(TypedDict):
    disable: bool
    allow: bool
    signing_key: str | None


class PrincipalInput(TypedDict):
    user_id: str | None
    team_id: str | None
    role: Literal["PROXY_ADMIN", "PROXY_ADMIN_VIEW_ONLY"] | None
    role_form: Literal["enum", "value"] | None


class TextValue(TypedDict):
    kind: Literal["text"]
    text: str


class NumberValue(TypedDict):
    kind: Literal["number"]
    number: int


class NullValue(TypedDict):
    kind: Literal["null"]


class OwnedValue(TypedDict):
    kind: Literal["owned"]
    original: str
    user: str
    team: str
    prefix: str
    suffix: str


class SealedValue(TypedDict):
    kind: Literal["sealed"]
    text: str
    prefix: str
    suffix: str


ValueSpec = TextValue | NumberValue | NullValue | OwnedValue | SealedValue


class AbsentNested(TypedDict):
    shape: Literal["absent"]


class NullNested(TypedDict):
    shape: Literal["null"]


class NumberNested(TypedDict):
    shape: Literal["number"]
    number: int


class MappingNested(TypedDict):
    shape: Literal["mapping"]
    id: ValueSpec


NestedSpec = AbsentNested | NullNested | NumberNested | MappingNested


class LifecycleEvent(TypedDict):
    kind: Literal["lifecycle"]
    response_id: ValueSpec
    top_id: ValueSpec | None


class GenericEvent(TypedDict):
    kind: Literal["generic"]
    top_id: ValueSpec | None
    nested: NestedSpec


EventSpec = LifecycleEvent | GenericEvent


class FieldInput(TypedDict):
    field: str
    value: ValueSpec


class AuthorizeInput(TypedDict):
    operation: Literal["authorize"]
    settings: SettingsInput
    principal: PrincipalInput
    # LiteLLM's literal leaves out the get/delete/cancel/list values the hook accepts
    call_type: CallTypesLiteral
    data: list[FieldInput]


class SuccessInput(TypedDict):
    operation: Literal["success"]
    settings: SettingsInput
    principal: PrincipalInput
    id: ValueSpec
    nested: NestedSpec
    route: str | None


class StreamInput(TypedDict):
    operation: Literal["stream"]
    settings: SettingsInput
    principal: PrincipalInput
    route: str | None
    events: list[EventSpec]


Input = AuthorizeInput | SuccessInput | StreamInput


class Outcome(TypedDict):
    kind: str
    detail: str | None
    status: int | None
    class_name: str | None


class StreamRun(NamedTuple):
    views: list[JsonValue]
    raw: list[str]
    advertised: list[str | None]


class ProxyServerStub(ModuleType):
    def __init__(self) -> None:
        super().__init__(PROXY_SERVER_NAME)
        self.master_key: str | None = None
        self.general_settings: dict[str, JsonValue] = {}


class SettingsReader:
    def __init__(self, general: Mapping[str, bool]) -> None:
        self.general = general

    def __call__(self) -> Mapping[str, bool]:
        return self.general


class SigningKeyReader:
    def __init__(self, key: str | None) -> None:
        self.key = key

    def __call__(self) -> str | None:
        return self.key


@contextmanager
def proxy_salt(signing_key: str | None) -> Generator[None]:
    saved_salt = os.environ.pop(SALT_ENV, None)
    if signing_key is not None:
        os.environ[SALT_ENV] = signing_key
    saved_module = sys.modules.get(PROXY_SERVER_NAME)
    sys.modules[PROXY_SERVER_NAME] = ProxyServerStub()
    try:
        yield
    finally:
        if saved_module is None:
            sys.modules.pop(PROXY_SERVER_NAME, None)
        else:
            sys.modules[PROXY_SERVER_NAME] = saved_module
        os.environ.pop(SALT_ENV, None)
        if saved_salt is not None:
            os.environ[SALT_ENV] = saved_salt


def response_hook(settings: SettingsInput) -> ResponsesIDSecurity:
    general: Mapping[str, bool] = {
        "disable_responses_id_security": settings["disable"],
        "allow_unmanaged_response_ids": settings["allow"],
    }
    return ResponsesIDSecurity(
        general_settings_reader=SettingsReader(general),
        signing_key_reader=SigningKeyReader(settings["signing_key"]),
    )


def principal_user(principal: PrincipalInput, route: str | None) -> UserAPIKeyAuth:
    role = principal["role"]
    if role is None:
        return UserAPIKeyAuth(
            user_id=principal["user_id"],
            team_id=principal["team_id"],
            request_route=route,
        )
    member = ROLE_MEMBERS[role]
    if principal["role_form"] == "enum":
        return UserAPIKeyAuth(
            user_id=principal["user_id"],
            team_id=principal["team_id"],
            user_role=member,
            request_route=route,
        )
    return UserAPIKeyAuth.model_construct(
        user_id=principal["user_id"],
        team_id=principal["team_id"],
        user_role=member.value,
        request_route=route,
    )


# LiteLLM's encrypt_value_helper has no return annotation, so the helper names its type once
ENCRYPT_VALUE: Final[Callable[[str], str]] = cast(Callable[[str], str], encrypt_value_helper)


def materialize(spec: ValueSpec) -> JsonValue:
    if spec["kind"] == "number":
        return spec["number"]
    if spec["kind"] == "text":
        return spec["text"]
    if spec["kind"] == "null":
        return None
    if spec["kind"] == "owned":
        payload = MANAGED_TEMPLATE.format(spec["original"], spec["user"], spec["team"])
        return f"{spec['prefix']}resp_{ENCRYPT_VALUE(payload)}{spec['suffix']}"
    return f"{spec['prefix']}resp_{ENCRYPT_VALUE(spec['text'])}{spec['suffix']}"


def text_or_none(value: JsonValue) -> str | None:
    try:
        return TEXT.validate_python(value, strict=True)
    except ValidationError:
        return None


def nonempty_text(value: JsonValue) -> bool:
    text = text_or_none(value)
    return text is not None and text != ""


def json_text(value: JsonValue) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def decrypted_text(ciphertext: str) -> str:
    plaintext = decrypt_value_helper(
        value=ciphertext,
        key="response_id",
        return_original_value=True,
    )
    if plaintext is None:
        return ""
    return plaintext


def project_text(text: str) -> str:
    if "resp_" not in text:
        return f"plain:{text}"
    segment = text.split("resp_")[1]
    plaintext = decrypted_text(segment)
    if plaintext == segment:
        return f"plain:{text}"
    if not plaintext.startswith("litellm_proxy"):
        return f"decrypted:{plaintext}"
    response_part, separator, remainder = plaintext.partition(";")
    if not separator:
        return "fallback"
    user_part, separator, remainder = remainder.partition(";")
    if not separator:
        return "malformed"
    team_part = remainder.partition(";")[0]
    original = response_part.split("response_id:")[-1]
    user = user_part.split("user_id:")[-1]
    team = team_part.split("team_id:")[-1]
    return f"managed:{original}|{user}|{team}"


def project(value: JsonValue) -> str:
    text = text_or_none(value)
    if text is None:
        return f"json:{json_text(value)}"
    return project_text(text)


def project_nested(value: JsonValue) -> str:
    try:
        mapping = MAPPING.validate_python(value, strict=True)
    except ValidationError:
        return project(value)
    if "id" not in mapping:
        return project(value)
    return project(mapping["id"])


def raw_nested_id(value: JsonValue) -> str | None:
    try:
        mapping = MAPPING.validate_python(value, strict=True)
    except ValidationError:
        return None
    if "id" not in mapping:
        return None
    return text_or_none(mapping["id"])


def advertised_id(dumped: dict[str, JsonValue]) -> str | None:
    if "id" in dumped:
        top = text_or_none(dumped["id"])
        if top is not None:
            return top
    if "response" not in dumped:
        return None
    return raw_nested_id(dumped["response"])


def nested_value(spec: NestedSpec) -> JsonValue:
    if spec["shape"] == "number":
        return spec["number"]
    if spec["shape"] == "mapping":
        return {"id": materialize(spec["id"])}
    return None


def event_chunk(spec: EventSpec) -> EventBase:
    if spec["kind"] == "lifecycle":
        source: dict[str, JsonValue] = {
            "type": "response.created",
            "response": {
                "id": TEXT.validate_python(materialize(spec["response_id"]), strict=True),
                "created_at": 0,
                "output": [],
            },
        }
        if spec["top_id"] is not None:
            source["id"] = materialize(spec["top_id"])
        return ResponseCreatedEvent.model_validate(source)
    generic: dict[str, JsonValue] = {"type": "response.custom_event"}
    if spec["top_id"] is not None:
        generic["id"] = materialize(spec["top_id"])
    nested_spec = spec["nested"]
    if nested_spec["shape"] != "absent":
        generic["response"] = nested_value(nested_spec)
    return EventBase.model_validate(generic)


async def chunk_source(chunks: list[EventBase]) -> AsyncGenerator[EventBase, None]:
    for chunk in chunks:
        yield chunk


async def collect_chunks(stream: AsyncGenerator[EventBase, None]) -> list[EventBase]:
    collected = [chunk async for chunk in stream]
    return collected


def top_view(dumped: dict[str, JsonValue]) -> str | None:
    if "id" not in dumped:
        return None
    return project(dumped["id"])


def nested_view(dumped: dict[str, JsonValue], spec: EventSpec) -> str | None:
    if "response" not in dumped:
        return None
    response = dumped["response"]
    if spec["kind"] == "lifecycle":
        raw = raw_nested_id(response)
        if raw is None:
            return None
        return project(raw)
    return project_nested(response)


def shared_advertisement(advertised: list[str | None]) -> bool:
    if not advertised:
        return False
    head = advertised[0]
    if head is None:
        return False
    return all(value == head for value in advertised)


def stream_run(hook: ResponsesIDSecurity, user: UserAPIKeyAuth, specs: list[EventSpec]) -> StreamRun:
    chunks = [event_chunk(spec) for spec in specs]
    stream = hook.async_post_call_streaming_iterator_hook(user, chunk_source(chunks), {})
    produced = asyncio.run(collect_chunks(stream))
    views: list[JsonValue] = []
    raw: list[str] = []
    advertised: list[str | None] = []
    for spec, chunk in zip(specs, produced, strict=True):
        text = chunk.model_dump_json()
        dumped = MAPPING.validate_json(text)
        views.append({"top": top_view(dumped), "nested": nested_view(dumped, spec)})
        raw.append(text)
        advertised.append(advertised_id(dumped))
    return StreamRun(views, raw, advertised)


def addressed_field(call_type: str) -> str | None:
    if call_type == "aresponses":
        return "previous_response_id"
    if call_type in {
        "aget_responses",
        "adelete_responses",
        "acancel_responses",
        "alist_input_items",
    }:
        return "response_id"
    return None


def project_field(data: dict[str, JsonValue], field_name: str | None) -> str | None:
    if field_name is None or field_name not in data:
        return None
    return project(data[field_name])


def call_answer(data: dict[str, JsonValue], field_name: str | None, outcome: Outcome) -> JsonValue:
    return {
        "call": {
            "kind": outcome["kind"],
            "detail": outcome["detail"],
            "type": None,
            "param": None,
            "code": None,
            "status": outcome["status"],
            "field": project_field(data, field_name),
            "retained": project_field(data, ADDRESSED_RESPONSE_ID_FIELD),
            "class": outcome["class_name"],
        },
    }


def addressed_present(data: dict[str, JsonValue], field_name: str | None) -> bool:
    if nonempty_text(data.get(ADDRESSED_RESPONSE_ID_FIELD)):
        return True
    if field_name is None:
        return False
    return nonempty_text(data.get(field_name))


def accepted_outcome(
    data: dict[str, JsonValue],
    field_name: str | None,
    result: Exception | str | dict[str, JsonValue] | None,
) -> Outcome:
    if result is None:
        return {"kind": "not_responses_call", "detail": None, "status": None, "class_name": None}
    kind = "authorized" if addressed_present(data, field_name) else "unchanged"
    return {"kind": kind, "detail": None, "status": None, "class_name": None}


def forbidden_kind(detail: str) -> str:
    if detail.startswith("Forbidden. This response id was not issued"):
        return "unmanaged_forbidden"
    if detail.startswith("Forbidden. The response id is not associated with the user"):
        return "user_forbidden"
    return "team_forbidden"


def authorize_answer(case: AuthorizeInput) -> JsonValue:
    settings = case["settings"]
    with proxy_salt(settings["signing_key"]):
        hook = response_hook(settings)
        user = principal_user(case["principal"], None)
        data = {field["field"]: materialize(field["value"]) for field in case["data"]}
        field_name = addressed_field(case["call_type"])
        try:
            result = asyncio.run(
                hook.async_pre_call_hook(user, DualCache(), data, case["call_type"])
            )
        except HTTPException as error:
            return call_answer(data, field_name, refusal_outcome(error))
        except IndexError as error:
            return call_answer(data, field_name, crash_outcome(error))
        return call_answer(data, field_name, accepted_outcome(data, field_name, result))


def refusal_outcome(error: HTTPException) -> Outcome:
    return {
        "kind": forbidden_kind(error.detail),
        "detail": error.detail,
        "status": error.status_code,
        "class_name": None,
    }


def crash_outcome(error: IndexError) -> Outcome:
    return {
        "kind": "internal_error",
        "detail": None,
        "status": 500,
        "class_name": type(error).__name__,
    }


def success_response(id_spec: ValueSpec, nested_spec: NestedSpec) -> ResponsesAPIResponse:
    source: dict[str, JsonValue] = {
        "id": TEXT.validate_python(materialize(id_spec), strict=True),
        "created_at": 0,
        "output": [],
    }
    if nested_spec["shape"] != "absent":
        source["response"] = nested_value(nested_spec)
    return ResponsesAPIResponse.model_validate(source)


def success_answer(case: SuccessInput) -> JsonValue:
    settings = case["settings"]
    with proxy_salt(settings["signing_key"]):
        hook = response_hook(settings)
        user = principal_user(case["principal"], case["route"])
        response = success_response(case["id"], case["nested"])
        asyncio.run(hook.async_post_call_success_hook({}, user, response))
        dumped = MAPPING.validate_json(response.model_dump_json())
        nested = None
        if "response" in dumped:
            nested = project_nested(dumped["response"])
        return {"response": {"id": project(dumped["id"]), "nested": nested}}


def stream_answer(case: StreamInput) -> JsonValue:
    settings = case["settings"]
    with proxy_salt(settings["signing_key"]):
        hook = response_hook(settings)
        user = principal_user(case["principal"], case["route"])
        first = stream_run(hook, user, case["events"])
        second = stream_run(hook, user, case["events"])
        return {
            "stream": {
                "events": first.views,
                "shared": shared_advertisement(first.advertised),
                "second_request_differs": first.raw != second.raw,
            },
        }


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "authorize":
        return authorize_answer(input_value)
    if input_value["operation"] == "success":
        return success_answer(input_value)
    return stream_answer(input_value)
