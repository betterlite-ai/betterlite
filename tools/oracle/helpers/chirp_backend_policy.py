from collections.abc import AsyncIterable, AsyncIterator
from typing import Literal, TypedDict

from litellm.llms.vertex_ai.audio_transcription import realtime_backend
from litellm.llms.vertex_ai.audio_transcription.realtime_transformation import (
    SpeechStreamingTarget,
)
from pydantic import JsonValue as Json
from websockets.exceptions import ConnectionClosed

_TURN_RESULT, _TURN_DISCARDED_EVENT, _STREAM_FAILURE, _CLOSED, _RECOGNIZE_STREAM = realtime_backend._TurnResult, realtime_backend._TurnDiscardedEvent, realtime_backend._StreamFailure, realtime_backend._Closed, realtime_backend._RecognizeStream  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/audio_transcription/realtime_backend.py#L82-L96


class Input(TypedDict):
    kind: Literal["deadline", "deliver"]
    elapsed_seconds: float
    speech_active: bool
    item_kind: Literal["event", "turn_result", "discard_marker", "stream_failure", "closed"]
    turn: int
    event: str
    reason: str
    discarded_turns: list[int]


class IdleTransport:
    async def close(self) -> None: ...


class IdleClient:
    @property
    def transport(self) -> IdleTransport:
        return IdleTransport()

    async def streaming_recognize(
        self, requests: AsyncIterator[bytes] | None = None
    ) -> AsyncIterable[bytes]:
        del requests
        raise NotImplementedError("the chirp_backend_policy helper never opens a stream")


class FixedClock:
    def __init__(self, seconds: float) -> None:
        self.seconds = seconds

    def __call__(self) -> float:
        return self.seconds


class UnreachableToken:
    async def __call__(self) -> str:
        raise NotImplementedError("the chirp_backend_policy helper never opens a stream")


def build_target() -> SpeechStreamingTarget:
    return SpeechStreamingTarget(
        api_endpoint="speech.googleapis.com",
        recognizer="projects/project/locations/us-central1/recognizers/_",
        resolve_access_token=UnreachableToken(),
    )


class PolicyBackend(realtime_backend.SpeechStreamingBackend):
    def expired(self, *, speech_active: bool) -> bool:
        stream = _RECOGNIZE_STREAM(
            client=IdleClient(),
            request_type=bytes,
            first_request=None,
            opened_at=0.0,
            turn=0,
        )
        stream.speech_active = speech_active
        return self._expired(stream)

    def deliver(self, input_value: Input) -> str | None:
        self._discarded_turns = frozenset(input_value["discarded_turns"])
        match input_value["item_kind"]:
            case "event":
                return self._deliverable(input_value["event"])
            case "turn_result":
                return self._deliverable(
                    _TURN_RESULT(turn=input_value["turn"], event=input_value["event"])
                )
            case "discard_marker":
                return self._deliverable(
                    _TURN_DISCARDED_EVENT(
                        turn=input_value["turn"], event=input_value["event"]
                    )
                )
            case "stream_failure":
                return self._deliverable(_STREAM_FAILURE(reason=input_value["reason"]))
            case "closed":
                return self._deliverable(_CLOSED())

    def discarded(self) -> list[Json]:
        turns: list[Json] = []
        turns.extend(sorted(self._discarded_turns))
        return turns


def deadline_answer(input_value: Input) -> Json:
    backend = PolicyBackend(build_target(), clock=FixedClock(input_value["elapsed_seconds"]))
    return {"expired": backend.expired(speech_active=input_value["speech_active"])}


def deliver_answer(input_value: Input) -> Json:
    backend = PolicyBackend(build_target())
    outcome: Json
    try:
        event = backend.deliver(input_value)
    except ConnectionClosed as closed:
        frame = closed.rcvd
        outcome = (
            {"kind": "closed", "code": 1000, "reason": ""}
            if frame is None
            else {"kind": "closed", "code": frame.code, "reason": frame.reason}
        )
    else:
        outcome = (
            {"kind": "suppressed"}
            if event is None
            else {"kind": "delivered", "event": event}
        )
    return {"outcome": outcome, "discarded_turns": backend.discarded()}


def run(input_value: Input) -> Json:
    if input_value["kind"] == "deadline":
        return deadline_answer(input_value)
    return deliver_answer(input_value)
