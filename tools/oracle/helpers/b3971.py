import asyncio
import json
from dataclasses import dataclass, field
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

from litellm.integrations.posthog import PostHogLogger
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

OK_STATUS = 200
FAILURE_STATUS = 400
POSTHOG_SOURCE = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/posthog.py"


class SettingsInput(TypedDict):
    api_key: str
    api_url: str
    mock: bool


class EventInput(TypedDict):
    payload: dict[str, Json]
    dynamic_params: NotRequired[dict[str, Json]]


class ReplyInput(TypedDict):
    status: int


class Input(TypedDict):
    operation: Literal[
        "init", "event_payload", "credentials", "sync_send", "async_queue", "async_batch",
        "async_init", "exit_flush",
    ]
    settings: SettingsInput
    metadata: NotRequired[dict[str, Json]]
    events: NotRequired[list[EventInput]]
    replies: NotRequired[list[ReplyInput]]


LoggerKwargs: TypeAlias = dict[str, Json]


class QueuedEntry(TypedDict):
    event: dict[str, Json]
    api_key: str
    api_url: str


class ResponseView(Protocol):
    status_code: int
    text: str

    def raise_for_status(self) -> None: ...


class SyncClientView(Protocol):
    def post(self, url: str, content: bytes, headers: dict[str, str]) -> ResponseView: ...


class AsyncClientView(Protocol):
    async def post(
        self, url: str, content: bytes, headers: dict[str, str]
    ) -> ResponseView: ...


class LoggerView(Protocol):
    POSTHOG_API_KEY: str
    posthog_host: str
    capture_url: str
    batch_size: int
    flush_interval: float
    max_queue_size: int
    is_mock_mode: bool
    log_queue: list[QueuedEntry]
    _async_initialized: bool
    flush_lock: asyncio.Lock | None
    sync_client: SyncClientView
    async_client: AsyncClientView

    def create_posthog_event_payload(self, kwargs: LoggerKwargs) -> dict[str, Json]: ...

    def _get_credentials_for_request(
        self, kwargs: LoggerKwargs
    ) -> tuple[str | None, str | None]: ...

    def log_success_event(
        self, kwargs: LoggerKwargs, response_obj: None, start_time: float, end_time: float
    ) -> None: ...

    async def async_log_success_event(
        self, kwargs: LoggerKwargs, response_obj: None, start_time: float, end_time: float
    ) -> None: ...

    async def flush_queue(self) -> None: ...

    def _flush_on_exit(self) -> None: ...


class RecordingResponse(ResponseView):
    def __init__(self, status: int) -> None:
        self.status_code = status
        self.text = ""

    @override
    def raise_for_status(self) -> None:
        if self.status_code >= FAILURE_STATUS:
            raise RuntimeError(f"status {self.status_code}")


class Recorder:
    def __init__(self, replies: list[ReplyInput]) -> None:
        self.replies = list(replies)
        self.requests: list[Json] = []
        self.sync = RecordingSync(self)
        self.async_ = RecordingAsync(self)

    def respond(
        self, url: str, content: bytes, headers: dict[str, str]
    ) -> ResponseView:
        body = cast("dict[str, Json]", json.loads(content))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/posthog.py#L412-L413
        self.requests.append(
            {
                "url": url,
                "headers": {name.lower(): value for name, value in headers.items()},
                "body": body,
            }
        )
        status = self.replies.pop(0)["status"] if self.replies else OK_STATUS
        return RecordingResponse(status)


class RecordingSync(SyncClientView):
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    @override
    def post(self, url: str, content: bytes, headers: dict[str, str]) -> ResponseView:
        return self.recorder.respond(url, content, headers)


class RecordingAsync(AsyncClientView):
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    @override
    async def post(
        self, url: str, content: bytes, headers: dict[str, str]
    ) -> ResponseView:
        return self.recorder.respond(url, content, headers)


@dataclass
class Outcome:
    built: list[Json] = field(default_factory=list)
    credentials: Json = None


def build_logger() -> LoggerView:
    # LiteLLM leaves PostHogLogger and every method on it unannotated
    return cast("LoggerView", PostHogLogger())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/posthog.py#L56-L94


def kwargs_for(event: EventInput, metadata: dict[str, Json] | None) -> LoggerKwargs:
    kwargs: LoggerKwargs = {
        "standard_logging_object": event["payload"],
        "litellm_params": {"metadata": {} if metadata is None else metadata},
    }
    dynamic = event.get("dynamic_params")
    if dynamic is not None:
        kwargs["standard_callback_dynamic_params"] = dynamic
    return kwargs


def async_state(logger: LoggerView) -> Json:
    return {
        "initialized": logger._async_initialized,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/posthog.py#L393-L402
        "flush_lock": logger.flush_lock is not None,
    }


def queue_value(logger: LoggerView) -> Json:
    entries: list[Json] = [
        {
            "event": item["event"],
            "api_key": item["api_key"],
            "api_url": item["api_url"],
        }
        for item in logger.log_queue
    ]
    return {"length": len(entries), "entries": entries}


def credentials_value(logger: LoggerView, kwargs: LoggerKwargs) -> Json:
    api_key, api_url = logger._get_credentials_for_request(kwargs)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/posthog.py#L315-L339
    return {"api_key": api_key, "api_url": api_url}


async def queue_all(
    logger: LoggerView, events: list[EventInput], metadata: dict[str, Json] | None
) -> None:
    for event in events:
        await logger.async_log_success_event(
            kwargs_for(event, metadata), None, 0.0, 0.0
        )


async def perform(
    operation: str,
    logger: LoggerView,
    events: list[EventInput],
    metadata: dict[str, Json] | None,
) -> Outcome:
    outcome = Outcome()
    if operation == "event_payload":
        outcome.built.extend(
            logger.create_posthog_event_payload(kwargs_for(event, metadata))
            for event in events
        )
    elif operation == "credentials":
        first = events[0] if events else None
        if first is not None:
            outcome.credentials = credentials_value(logger, kwargs_for(first, metadata))
    elif operation == "sync_send":
        for event in events:
            logger.log_success_event(kwargs_for(event, metadata), None, 0.0, 0.0)
    elif operation == "async_queue":
        await queue_all(logger, events, metadata)
    elif operation == "async_batch":
        await queue_all(logger, events, metadata)
        await logger.flush_queue()
    elif operation == "async_init":
        await queue_all(logger, events[:1], metadata)
    elif operation == "exit_flush":
        await queue_all(logger, events, metadata)
        logger._flush_on_exit()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/posthog.py#L420-L470
    return outcome


def report(
    logger: LoggerView,
    recorder: Recorder,
    outcome: Outcome,
    before: Json,
) -> Json:
    return {
        "settings": {
            "host": logger.posthog_host,
            "capture_url": logger.capture_url,
            "batch_size": logger.batch_size,
            "flush_interval_seconds": int(logger.flush_interval),
            "mock_mode": logger.is_mock_mode,
            "max_queue_size": logger.max_queue_size,
        },
        "events": outcome.built,
        "credentials": outcome.credentials,
        "queue": queue_value(logger),
        "requests": recorder.requests,
        "async": async_state(logger),
        "async_before": before,
    }


def run(input_value: Input) -> Json:
    logger = build_logger()
    replies = input_value.get("replies")
    recorder = Recorder([] if replies is None else replies)
    logger.sync_client = recorder.sync
    logger.async_client = recorder.async_
    before = async_state(logger)
    raw_events = input_value.get("events")
    events = [] if raw_events is None else raw_events
    metadata = input_value.get("metadata")
    outcome = asyncio.run(
        perform(input_value["operation"], logger, events, metadata)
    )
    return report(logger, recorder, outcome, before)
