from collections.abc import Sequence
from typing import Literal, TypedDict

from litellm.litellm_core_utils.realtime_streaming import (
    RealTimeStreaming,
    client_sent_openai_beta_realtime_header,
)
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class HeaderCase(TypedDict):
    headers: list[tuple[str, str]]
    latin1: bool
    scope_error: bool


class Input(TypedDict):
    function: Literal["header_detection", "session_remap", "ga_events_for_beta"]
    websockets: list[HeaderCase]
    sessions: list[dict[str, JsonValue]]
    events: list[dict[str, JsonValue]]


class Scope(TypedDict, total=False):
    headers: Sequence[tuple[bytes | str, bytes | str]]


class ScopeWebSocket:
    scope: Scope

    def __init__(self, headers: Sequence[tuple[bytes | str, bytes | str]]) -> None:
        self.scope = {"headers": headers}


class FailingScopeWebSocket:
    @property
    def scope(self) -> Scope:
        raise RuntimeError("the client websocket has no scope")


def header_pair(text: str, *, latin1: bool) -> bytes | str:
    if latin1:
        return text.encode("latin-1")
    return text


def header_answer(case: HeaderCase) -> bool:
    if case["scope_error"]:
        return client_sent_openai_beta_realtime_header(FailingScopeWebSocket())
    pairs = [
        (header_pair(name, latin1=case["latin1"]), header_pair(value, latin1=case["latin1"]))
        for name, value in case["headers"]
    ]
    return client_sent_openai_beta_realtime_header(ScopeWebSocket(pairs))


def session_answer(session: dict[str, JsonValue]) -> JsonValue:
    return JSON.validate_python(RealTimeStreaming._remap_beta_session_to_ga(session))  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L1177-L1258


def event_answer(event: dict[str, JsonValue]) -> JsonValue:
    return JSON.validate_python(RealTimeStreaming._translate_event_to_beta(event))  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L1260-L1295


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "header_detection":
        return {"answers": [header_answer(case) for case in input_value["websockets"]]}
    if function == "session_remap":
        return {"answers": [session_answer(session) for session in input_value["sessions"]]}
    return {"answers": [event_answer(event) for event in input_value["events"]]}
