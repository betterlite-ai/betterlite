import asyncio
from collections.abc import Mapping
from typing import Final, Literal, NotRequired, TypedDict
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
JSON_OBJECT: Final = TypeAdapter(dict[str, JsonValue])
CREDENTIALS: Final = TypeAdapter(dict[str, JsonValue] | None)
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


class Lookup(TypedDict):
    model_id: str
    team_id: str | None


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
        self.lookups: list[Lookup] = []
        super().__init__(model_list=[])
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
    def get_deployment_credentials_with_provider(  # ast-grep-ignore: no-long-names
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
        if info is None:
            return None
        return deployment({"model_name": model_id, "model_info": info})


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


def text_fields(snapshot: Mapping[str, JsonValue]) -> dict[str, str]:
    texts: dict[str, str] = {}
    for name, value in snapshot.items():
        match value:
            case str():
                texts[name] = value
            case None | bool() | int() | float() | list() | dict():
                continue
    return texts


def extract_answer(call: ExtractCall) -> JsonValue:
    embedded, param = common_utils.extract_model_from_sources(
        file_id=call["file_id"],
        request=request_for(call["hints"]),
        data={"model": call["hints"]["body"]},
    )
    return JSON.validate_python({"function": "extract", "embedded": embedded, "param": param})


def routing_answer(call: RoutingCall) -> JsonValue:
    router = scripted_router(call)
    use_model_routing = False
    model: str | None = None
    original_file_id: str | None = None
    credentials: dict[str, JsonValue] | None = None
    error: dict[str, JsonValue] | None = None
    try:
        use_model_routing, model, original_file_id, found = asyncio.run(
            common_utils.handle_model_based_routing(
                file_id=call["file_id"],
                request=request_for(call["hints"]),
                llm_router=router,
                data={"model": call["hints"]["body"]},
                user_api_key_dict=UserAPIKeyAuth(models=[], team_models=[], team_id=None),
                check_file_id_encoding=call["check_file_id_encoding"],
            )
        )
        credentials = CREDENTIALS.validate_python(found)
    except HTTPException as raised:
        error = {
            "status": raised.status_code,
            "detail": DETAIL.validate_python(raised.detail)["error"],
        }
    return JSON.validate_python(
        {
            "function": "routing",
            "use_model_routing": use_model_routing,
            "model": model,
            "original_file_id": original_file_id,
            "credentials": credentials,
            "error": error,
        }
    )


def team_answer(call: TeamCall) -> JsonValue:
    router = scripted_router(call)
    found = common_utils.get_team_provider_credentials(
        llm_router=router,
        user_api_key_dict=token(call["key"]),
        custom_llm_provider=call["provider"],
    )
    return JSON.validate_python(
        {
            "function": "team_credentials",
            "credentials": CREDENTIALS.validate_python(found),
            "lookups": [] if router is None else router.lookups,
        }
    )


def apply_answer(call: ApplyCall) -> JsonValue:
    router = scripted_router(call)
    data = dict(call["data"])
    common_utils.apply_team_provider_credentials(
        data=data,
        llm_router=router,
        user_api_key_dict=token(call["key"]),
        custom_llm_provider=call["provider"],
    )
    return JSON.validate_python(
        {"function": "apply_team_credentials", "data": JSON_OBJECT.validate_python(data)}
    )


def internal_answer(call: InternalCall) -> JsonValue:
    router = scripted_router(call)
    data = dict(call["data"])
    common_utils.add_internal_model_credentials(
        data=data, llm_router=router, model_id=call["model_id"]
    )
    snapshot: dict[str, str] | None = None
    if SNAPSHOT_KEY in data:
        snapshot = text_fields(SNAPSHOT.validate_python(data[SNAPSHOT_KEY]))
    return JSON.validate_python(
        {
            "function": "internal_credentials",
            "data": {name: value for name, value in data.items() if name != SNAPSHOT_KEY},
            "internal_key": SNAPSHOT_KEY in data,
            "snapshot": snapshot,
        }
    )


def info_answer(call: InfoCall) -> JsonValue:
    router = scripted_router(call)
    data = dict(call["data"])
    common_utils.add_deployment_model_info(
        data=data, llm_router=router, model_id=call["model_id"]
    )
    return JSON.validate_python(
        {"function": "deployment_info", "data": JSON_OBJECT.validate_python(data)}
    )


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
