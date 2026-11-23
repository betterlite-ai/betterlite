from collections.abc import Mapping
from typing import Final, Literal, NotRequired, Protocol, TypedDict

from litellm.proxy.openai_files_endpoints import common_utils
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

JSON: Final = TypeAdapter(JsonValue)
SNAPSHOT: Final = TypeAdapter(Mapping[str, JsonValue])
DETAIL: Final = TypeAdapter(dict[str, str])

SNAPSHOT_KEY: Final = "_litellm_internal_model_credentials"


class LookupCall(TypedDict):
    function: Literal["lookup"]
    model_id: str
    operation_context: str
    router: NotRequired[dict[str, JsonValue] | None]


class MergeCall(TypedDict):
    function: Literal["merge"]
    data: dict[str, JsonValue]
    credentials: dict[str, JsonValue]
    file_id: NotRequired[str | None]
    include_internal_credentials: bool


class Input(TypedDict):
    calls: list[LookupCall | MergeCall]


class ErrorView(TypedDict):
    status: int
    detail: str


class PortView(TypedDict):
    model_id: str
    team_id: str | None


class LookupAnswer(TypedDict):
    function: Literal["lookup"]
    credentials: dict[str, JsonValue] | None
    port: PortView | None
    error: ErrorView | None


class MergeAnswer(TypedDict):
    function: Literal["merge"]
    data: dict[str, JsonValue]
    snapshot: dict[str, str] | None
    internal_key: bool


class ScriptedRouter:
    def __init__(self, answer: dict[str, JsonValue] | None) -> None:
        self.answer = answer
        self.asked: PortView | None = None

    def get_deployment_credentials_with_provider(  # ast-grep-ignore: no-long-names
        self, model_id: str, team_id: str | None = None
    ) -> dict[str, JsonValue] | None:
        self.asked = {"model_id": model_id, "team_id": team_id}
        return self.answer


class CommonUtils(Protocol):
    def get_credentials_for_model(
        self, llm_router: ScriptedRouter | None, model_id: str, operation_context: str = ...
    ) -> dict[str, JsonValue] | None: ...

    def prepare_data_with_credentials(
        self,
        data: dict[str, JsonValue],
        credentials: dict[str, JsonValue],
        *,
        file_id: str | None = ...,
        include_internal_credentials: bool = ...,
    ) -> None: ...


def router_argument(call: LookupCall) -> ScriptedRouter | None:
    if "router" not in call:
        return None
    return ScriptedRouter(call["router"])


def asked_port(router: ScriptedRouter | None) -> PortView | None:
    if router is None:
        return None
    return router.asked


def lookup_answer(call: LookupCall, utils: CommonUtils = common_utils) -> JsonValue:
    router = router_argument(call)
    error: ErrorView | None = None
    credentials: dict[str, JsonValue] | None = None
    try:
        credentials = utils.get_credentials_for_model(
            router, call["model_id"], call["operation_context"]
        )
    except HTTPException as raised:
        error = {
            "status": raised.status_code,
            "detail": DETAIL.validate_python(raised.detail)["error"],
        }
    answer = LookupAnswer(
        function="lookup", credentials=credentials, port=asked_port(router), error=error
    )
    return JSON.validate_python(answer)


def text_field(value: JsonValue) -> str | None:
    match value:
        case str():
            return value
        case None | bool() | int() | float() | list() | dict():
            return None


def text_fields(snapshot: Mapping[str, JsonValue]) -> dict[str, str]:
    texts: dict[str, str] = {}
    for name, value in snapshot.items():
        text = text_field(value)
        if text is not None:
            texts[name] = text
    return texts


def merge_answer(call: MergeCall, utils: CommonUtils = common_utils) -> JsonValue:
    data = dict(call["data"])
    utils.prepare_data_with_credentials(
        data,
        dict(call["credentials"]),
        file_id=call.get("file_id"),
        include_internal_credentials=call["include_internal_credentials"],
    )
    snapshot: dict[str, str] | None = None
    if call["include_internal_credentials"]:
        snapshot = text_fields(SNAPSHOT.validate_python(data[SNAPSHOT_KEY]))
    answer = MergeAnswer(
        function="merge",
        data={name: value for name, value in data.items() if name != SNAPSHOT_KEY},
        snapshot=snapshot,
        internal_key=SNAPSHOT_KEY in data and not call["include_internal_credentials"],
    )
    return JSON.validate_python(answer)


def answer(call: LookupCall | MergeCall) -> JsonValue:
    if call["function"] == "lookup":
        return lookup_answer(call)
    return merge_answer(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
