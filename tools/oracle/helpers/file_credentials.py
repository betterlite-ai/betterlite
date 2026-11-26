import asyncio
from collections.abc import Callable, Coroutine, Mapping
from typing import Final, Literal, NotRequired, TypedDict, cast
from urllib.parse import urlencode

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.openai_files_endpoints import common_utils
from litellm.router import Router
from litellm.types.router import Deployment, LiteLLM_Params
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from starlette.requests import Request
from starlette.types import Scope
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)
DETAIL: Final = TypeAdapter(dict[str, str])
SNAPSHOT: Final = TypeAdapter(Mapping[str, JsonValue])

SNAPSHOT_KEY: Final = "_litellm_internal_model_credentials"


class Hints(TypedDict):
    body: str | None
    query: str | None
    header: str | None


class KeyScript(TypedDict):
    models: list[str]
    team_models: list[str]
    team_id: str | None


class RowScript(TypedDict):
    model_name: str
    model_info: dict[str, JsonValue]


class RouterScript(TypedDict):
    model_names: list[str]
    access_groups: dict[str, list[str]]
    deployments: list[RowScript]
    credentials: dict[str, dict[str, JsonValue]]
    credential_deployments: dict[str, dict[str, JsonValue]]


class ExtractCall(TypedDict):
    function: Literal["extract"]
    file_id: str
    hints: Hints


class RoutingCall(TypedDict):
    function: Literal["routing"]
    file_id: str
    hints: Hints
    check_file_id_encoding: bool
    router: NotRequired[RouterScript]


class TeamCall(TypedDict):
    function: Literal["team_credentials"]
    provider: str
    key: KeyScript
    router: NotRequired[RouterScript]


class ApplyCall(TypedDict):
    function: Literal["apply_team_credentials"]
    provider: str
    key: KeyScript
    data: dict[str, JsonValue]
    router: NotRequired[RouterScript]


class InternalCall(TypedDict):
    function: Literal["internal_credentials"]
    model_id: str | None
    data: dict[str, JsonValue]
    router: RouterScript


class InfoCall(TypedDict):
    function: Literal["deployment_info"]
    model_id: str
    data: dict[str, JsonValue]
    router: NotRequired[RouterScript]


Call = ExtractCall | RoutingCall | TeamCall | ApplyCall | InternalCall | InfoCall
RouterCall = RoutingCall | TeamCall | ApplyCall | InternalCall | InfoCall


class Input(TypedDict):
    calls: list[Call]


EXTRACT = cast("Callable[[str, Request, dict[str, JsonValue] | None], tuple[str | None, str | None]]", common_utils.extract_model_from_sources)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L317
ROUTE = cast("Callable[[str, Request, Router | None, dict[str, JsonValue], UserAPIKeyAuth, bool], Coroutine[object, object, tuple[bool, str | None, str | None, dict[str, JsonValue] | None]]]", common_utils.handle_model_based_routing)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L647
TEAM = cast("Callable[[Router | None, UserAPIKeyAuth, str], dict[str, JsonValue] | None]", common_utils.get_team_provider_credentials)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L436
APPLY = cast("Callable[[dict[str, JsonValue], Router | None, UserAPIKeyAuth, str], None]", common_utils.apply_team_provider_credentials)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L556
INTERNAL = cast("Callable[[dict[str, JsonValue], Router, str | None], None]", common_utils.add_internal_model_credentials)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L578
INFO = cast("Callable[[dict[str, JsonValue], Router | None, str], None]", common_utils.add_deployment_model_info)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L603


def deployment(row: RowScript) -> Deployment:
    return Deployment(
        model_name=row["model_name"],
        litellm_params=LiteLLM_Params(model="openai/scripted"),
        model_info=row["model_info"],
    )


class ScriptedRouter(Router):
    def __init__(self, script: RouterScript) -> None:
        # the base router reads back the overrides below while it initializes
        self.script: RouterScript = script
        self.lookups: list[JsonValue] = []
        super().__init__(model_list=[])  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L774
        self.model_list = [deployment(row) for row in script["deployments"]]

    @override
    def get_model_names(self, team_id: str | None = None) -> list[str]:
        del team_id
        return self.script["model_names"]

    @override
    def get_model_access_groups(
        self,
        model_name: str | None = None,
        model_access_group: str | None = None,
        team_id: str | None = None,
    ) -> dict[str, list[str]]:
        del model_name, model_access_group, team_id
        return self.script["access_groups"]

    @override
    def get_deployment_credentials_with_provider(
        self, model_id: str, team_id: str | None = None
    ) -> dict[str, JsonValue] | None:
        self.lookups.append({"model_id": model_id, "team_id": team_id})
        return self.script["credentials"].get(model_id)

    @override
    def get_credential_deployment(
        self, model_id: str, team_id: str | None = None
    ) -> Deployment | None:
        del team_id
        info = self.script["credential_deployments"].get(model_id)
        return None if info is None else deployment({"model_name": model_id, "model_info": info})


def scripted_router(call: RouterCall) -> ScriptedRouter | None:
    if "router" not in call:
        return None
    return ScriptedRouter(call["router"])


def token(key: KeyScript) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        models=key["models"],
        team_models=key["team_models"],
        team_id=key["team_id"],
    )


def request_for(hints: Hints) -> Request:
    header = hints["header"]
    headers: list[tuple[bytes, bytes]] = (
        [] if header is None else [(b"x-litellm-model", header.encode())]
    )
    query = hints["query"]
    scope: Scope = {
        "type": "http",
        "method": "GET",
        "path": "/v1/files",
        "headers": headers,
        "query_string": ("" if query is None else urlencode({"model": query})).encode(),
    }
    return Request(scope=scope)


def text_fields(snapshot: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    texts: dict[str, JsonValue] = {}
    for name, value in snapshot.items():
        match value:
            case str():
                texts[name] = value
            case None | bool() | int() | float() | list() | dict():
                continue
    return texts


def routed_answer(
    routed: tuple[bool, str | None, str | None, dict[str, JsonValue] | None],
    error: dict[str, JsonValue] | None,
) -> JsonValue:
    use_model_routing, model, original_file_id, credentials = routed
    answer: dict[str, JsonValue] = {
        "function": "routing",
        "use_model_routing": use_model_routing,
        "model": model,
        "original_file_id": original_file_id,
        "credentials": credentials,
        "error": error,
    }
    return JSON.validate_python(answer)


def extract_answer(call: ExtractCall) -> JsonValue:
    embedded, param = EXTRACT(
        call["file_id"], request_for(call["hints"]), {"model": call["hints"]["body"]}
    )
    answer: dict[str, JsonValue] = {"function": "extract", "embedded": embedded, "param": param}
    return JSON.validate_python(answer)


def routing_answer(call: RoutingCall) -> JsonValue:
    try:
        routed = asyncio.run(
            ROUTE(
                call["file_id"],
                request_for(call["hints"]),
                scripted_router(call),
                {"model": call["hints"]["body"]},
                token({"models": [], "team_models": [], "team_id": None}),
                call["check_file_id_encoding"],
            )
        )
    except HTTPException as raised:
        failure: dict[str, JsonValue] = {
            "status": raised.status_code,
            "detail": DETAIL.validate_python(raised.detail)["error"],
        }
        return routed_answer((False, None, None, None), failure)
    return routed_answer(routed, None)


def team_answer(call: TeamCall) -> JsonValue:
    router = scripted_router(call)
    found = TEAM(router, token(call["key"]), call["provider"])
    lookups: list[JsonValue] = [] if router is None else router.lookups
    answer: dict[str, JsonValue] = {
        "function": "team_credentials",
        "credentials": found,
        "lookups": lookups,
    }
    return JSON.validate_python(answer)


def apply_answer(call: ApplyCall) -> JsonValue:
    data = dict(call["data"])
    APPLY(data, scripted_router(call), token(call["key"]), call["provider"])
    answer: dict[str, JsonValue] = {"function": "apply_team_credentials", "data": data}
    return JSON.validate_python(answer)


def internal_answer(call: InternalCall) -> JsonValue:
    data = dict(call["data"])
    router = scripted_router(call)
    if router is not None:
        INTERNAL(data, router, call["model_id"])
    installed = data.pop(SNAPSHOT_KEY, None)
    snapshot = None if installed is None else text_fields(SNAPSHOT.validate_python(installed))
    answer: dict[str, JsonValue] = {
        "function": "internal_credentials",
        "data": data,
        "internal_key": installed is not None,
        "snapshot": snapshot,
    }
    return JSON.validate_python(answer)


def info_answer(call: InfoCall) -> JsonValue:
    data = dict(call["data"])
    INFO(data, scripted_router(call), call["model_id"])
    answer: dict[str, JsonValue] = {"function": "deployment_info", "data": data}
    return JSON.validate_python(answer)


def answer(call: Call) -> JsonValue:
    if call["function"] == "extract":
        return extract_answer(call)
    if call["function"] == "routing":
        return routing_answer(call)
    if call["function"] == "team_credentials":
        return team_answer(call)
    if call["function"] == "apply_team_credentials":
        return apply_answer(call)
    if call["function"] == "internal_credentials":
        return internal_answer(call)
    return info_answer(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
