import asyncio
import json
from collections.abc import Callable, Coroutine
from typing import Final, Literal, NotRequired, Required, TypedDict, cast

import litellm
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.openai_files_endpoints import common_utils
from litellm.proxy.vector_store_files_endpoints import endpoints
from litellm.router import Router
from litellm.types.vector_stores import LiteLLM_ManagedVectorStore
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from starlette.requests import Request
from starlette.types import Message, Scope
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)


class RequestScript(TypedDict, total=False):
    method: str
    path: str
    query: str
    headers: list[tuple[str, str]]
    body: dict[str, JsonValue]


class Caller(TypedDict, total=False):
    team_id: str
    team_models: JsonValue
    metadata: dict[str, JsonValue]
    team_metadata: dict[str, JsonValue]
    user_role: str


class RoutingCall(TypedDict):
    function: Literal["unified_file_routing"]
    data: dict[str, JsonValue]
    file_id: str
    caller: Caller
    request: NotRequired[RequestScript]
    catalog: NotRequired[dict[str, dict[str, JsonValue]] | None]
    deny: NotRequired[list[str]]
    managed_files: NotRequired[bool | None]
    require_managed_files: NotRequired[bool]


class HintCall(TypedDict):
    function: Literal["model_hint_precedence", "team_model_fallback"]
    data: dict[str, JsonValue]
    request: NotRequired[RequestScript]
    caller: NotRequired[Caller | None]
    catalog: NotRequired[dict[str, dict[str, JsonValue]] | None]
    deny: NotRequired[list[str]]


class OverlayCall(TypedDict):
    function: Literal["unified_store_and_registry_overlay"]
    data: dict[str, JsonValue]
    vector_store_id: str
    managed_store: NotRequired[LiteLLM_ManagedVectorStore | None]
    registry: NotRequired[list[LiteLLM_ManagedVectorStore] | None]
    should_lookup_registry: NotRequired[bool | None]


class PermissionsCall(TypedDict):
    function: Literal["provider_and_index_permissions"]
    data: dict[str, JsonValue]
    vector_store_id: str
    caller: Caller
    request: NotRequired[RequestScript]


class ResponseScript(TypedDict, total=False):
    kind: Required[Literal["null", "dict", "object"]]
    fields: dict[str, JsonValue]
    id: str
    file_id: str


class RestoreCall(TypedDict):
    function: Literal["restore_file_identifiers"]
    original_file_id: str
    response: ResponseScript


Call = RoutingCall | HintCall | OverlayCall | PermissionsCall | RestoreCall


class Input(TypedDict):
    calls: list[Call]


class ScriptObject:
    def __init__(self, identifier: str | None, file_id: str | None) -> None:
        if identifier is not None:
            self.id = identifier
        if file_id is not None:
            self.file_id = file_id


class BodyReceiver:
    def __init__(self, body: bytes) -> None:
        self.body = body
        self.sent = False

    async def __call__(self) -> Message:
        if self.sent:
            return {"type": "http.disconnect"}
        self.sent = True
        return {"type": "http.request", "body": self.body, "more_body": False}


class ScriptedRouter(Router):
    def __init__(self, script: dict[str, dict[str, JsonValue]]) -> None:
        # the base router reads the override back while it initializes
        self.script = script
        self.lookups: list[JsonValue] = []
        super().__init__(model_list=[])  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L774

    @override
    def get_deployment_credentials_with_provider(  # ast-grep-ignore: no-long-names
        self, model_id: str, team_id: str | None = None
    ) -> dict[str, JsonValue] | None:
        self.lookups.append({"model_id": model_id, "team_id": team_id})
        return self.script.get(model_id)


class Checker:
    def __init__(self, *, allow: bool) -> None:
        self.allow = allow

    async def can_user_call_unified_file_id(
        self, unified_file_id: str, user_api_key_dict: UserAPIKeyAuth
    ) -> bool:
        del unified_file_id, user_api_key_dict
        return self.allow

    async def can_user_call_unified_object_id(  # ast-grep-ignore: no-long-names
        self, unified_object_id: str, user_api_key_dict: UserAPIKeyAuth
    ) -> bool:
        del unified_object_id, user_api_key_dict
        return self.allow


class ScriptedAuthorize:
    def __init__(self, denied: list[str] | None) -> None:
        self.denied = [] if denied is None else denied
        self.asked: list[JsonValue] = []

    async def __call__(
        self, model_id: str, llm_router: Router | None, user_api_key_dict: UserAPIKeyAuth
    ) -> None:
        del llm_router, user_api_key_dict
        self.asked.append(model_id)
        if model_id in self.denied:
            raise HTTPException(status_code=403, detail=f"the scripted grant denied `{model_id}`")


MANAGED_FILE = cast("Callable[[dict[str, JsonValue], str, Request, UserAPIKeyAuth, object | None, Router | None], Coroutine[object, object, tuple[dict[str, JsonValue], str | None]]]", endpoints._update_request_data_with_managed_file_id)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_files_endpoints/endpoints.py#L35-L171
HINT = cast("Callable[[dict[str, JsonValue], Request, Router | None, UserAPIKeyAuth | None], Coroutine[object, object, dict[str, JsonValue]]]", endpoints._update_request_data_with_model_routing_hint)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_files_endpoints/endpoints.py#L218-L313
OVERLAY = cast("Callable[[dict[str, JsonValue], str, Router | None, LiteLLM_ManagedVectorStore | None, bool], dict[str, JsonValue]]", endpoints._update_request_data_with_litellm_managed_vector_store_registry)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_files_endpoints/endpoints.py#L316-L408
REPLACE = cast("Callable[[object | None, str], object | None]", endpoints._replace_file_id_in_response)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_files_endpoints/endpoints.py#L174-L204


def error_view(raised: HTTPException) -> JsonValue:
    detail = TypeAdapter(str | dict[str, str]).validate_python(raised.detail)
    match detail:
        case str():
            text = detail
        case dict():
            text = detail["error"]
    return JSON.validate_python({"status": raised.status_code, "detail": text})


def request_for(script: RequestScript | None) -> Request:
    fields: RequestScript = {} if script is None else script
    method = fields.get("method")
    path = fields.get("path")
    query = fields.get("query")
    body = fields.get("body")
    pairs = fields.get("headers")
    headers = [] if pairs is None else [(n.lower().encode(), v.encode()) for n, v in pairs]
    scope: Scope = {
        "type": "http", "method": "GET" if method is None else method,
        "path": "/v1/vector_stores/vs_1/files" if path is None else path,
        "headers": headers, "query_string": b"" if query is None else query.encode(),
    }
    return Request(scope, BodyReceiver(b"" if body is None else json.dumps(body).encode()))


def model_answer(call: RoutingCall | HintCall) -> JsonValue:
    catalog = call.get("catalog")
    router = None if catalog is None else ScriptedRouter(catalog)
    caller = call.get("caller")
    grant = ScriptedAuthorize(call.get("deny"))
    endpoints.authorize_model_for_key = grant
    common_utils.authorize_model_for_key = grant
    data = dict(call["data"])
    restored: str | None = None
    error: JsonValue = None
    try:
        if call["function"] == "unified_file_routing":
            managed = call.get("managed_files")
            litellm.require_managed_files = call.get("require_managed_files") is True
            restored = asyncio.run(
                MANAGED_FILE(
                    data, call["file_id"], request_for(call.get("request")),
                    UserAPIKeyAuth.model_construct(**call["caller"]),
                    None if managed is None else Checker(allow=managed), router,
                )
            )[1]
        else:
            asyncio.run(
                HINT(
                    data, request_for(call.get("request")), router,
                    None if caller is None else UserAPIKeyAuth.model_construct(**caller),
                )
            )
    except HTTPException as raised:
        error = error_view(raised)
    finally:
        litellm.require_managed_files = False
    if call["function"] == "unified_file_routing":
        answer: dict[str, JsonValue] = {"data": data, "restored": restored, "error": error}
    else:
        answer = {"data": data, "authorizations": grant.asked, "error": error}
    answer["lookups"] = [] if router is None else router.lookups
    return JSON.validate_python(answer)


def restore_answer(call: RestoreCall) -> JsonValue:
    script = call["response"]
    if script["kind"] == "null":
        return JSON.validate_python({"response": None})
    if script["kind"] == "dict":
        fields = script.get("fields")
        document = {} if fields is None else dict(fields)
        REPLACE(document, call["original_file_id"])
        return JSON.validate_python({"response": document})
    identifier = script.get("id")
    file_id = script.get("file_id")
    response = ScriptObject(identifier, file_id)
    REPLACE(response, call["original_file_id"])
    restored: dict[str, JsonValue] = {}
    if identifier is not None:
        restored["id"] = response.id
    if file_id is not None:
        restored["file_id"] = response.file_id
    return JSON.validate_python({"response": restored})


def overlay_answer(call: OverlayCall) -> JsonValue:
    registry = call.get("registry")
    lookup = call.get("should_lookup_registry")
    stores = None if registry is None else VectorStoreRegistry(vector_stores=registry)
    litellm.vector_store_registry = stores
    data = dict(call["data"])
    try:
        OVERLAY(
            data, call["vector_store_id"], None, call.get("managed_store"),
            True if lookup is None else lookup,
        )
    finally:
        litellm.vector_store_registry = None
    return JSON.validate_python({"data": data, "error": None})


def permissions_answer(call: PermissionsCall) -> JsonValue:
    request = request_for(call.get("request"))
    provider = asyncio.run(endpoints._resolve_provider(data=call["data"], request=request))  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_files_endpoints/endpoints.py#L411-L431
    name = None if provider is None else provider.value
    try:
        endpoints._maybe_check_permissions(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_files_endpoints/endpoints.py#L434-L452
            provider=provider, vector_store_id=call["vector_store_id"],
            request=request, user_api_key_dict=UserAPIKeyAuth.model_construct(**call["caller"]),
        )
    except HTTPException as raised:
        return JSON.validate_python({"provider": None, "error": error_view(raised)})
    return JSON.validate_python({"provider": name, "error": None})


def answer(call: Call) -> JsonValue:
    if call["function"] == "restore_file_identifiers":
        return restore_answer(call)
    if call["function"] == "unified_store_and_registry_overlay":
        return overlay_answer(call)
    if call["function"] == "provider_and_index_permissions":
        return permissions_answer(call)
    return model_answer(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
