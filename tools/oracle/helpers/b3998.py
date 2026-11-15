import asyncio
import os
import re
from datetime import UTC, datetime
from typing import Literal, NotRequired, Protocol, TypedDict

import httpx
from litellm.integrations.gcs_bucket import gcs_bucket_mock_client
from litellm.integrations.gcs_bucket.gcs_bucket import GCSBucketLogger
from litellm.proxy import proxy_server
from pydantic import JsonValue, TypeAdapter

JSON_VALUE = TypeAdapter(JsonValue)
WAIT_SECONDS = 1.5
EPOCH_SECONDS = 0.0
# LiteLLM gates GCS logging on its paid tier, which betterlite does not have
proxy_server.premium_user = True


class Reply(TypedDict):
    status: int
    body: str


class Step(TypedDict):
    do: Literal[
        "event",
        "failure",
        "settings",
        "drain",
        "config",
        "group",
        "ndjson",
        "grouped",
        "send",
        "retrieve",
        "date",
        "flush",
        "wait",
    ]
    id: NotRequired[str]
    response_id: NotRequired[str]
    dynamic: NotRequired[dict[str, JsonValue]]
    metadata: NotRequired[dict[str, JsonValue]]


class Input(TypedDict):
    bucket: NotRequired[str]
    environment: NotRequired[dict[str, str]]
    payload: NotRequired[dict[str, JsonValue]]
    steps: list[Step]
    replies: NotRequired[list[Reply]]
    dynamic: NotRequired[dict[str, JsonValue]]
    request_id: NotRequired[str]
    start_time: NotRequired[int]
    timestamp: NotRequired[int]


class EventSink(Protocol):
    async def async_log_success_event(
        self,
        kwargs: dict[str, JsonValue],
        response_obj: JsonValue,
        start_time: float,
        end_time: float,
    ) -> None: ...

    async def async_log_failure_event(
        self,
        kwargs: dict[str, JsonValue],
        response_obj: JsonValue,
        start_time: float,
        end_time: float,
    ) -> None: ...


class Background(Protocol):
    async def async_send_batch(self) -> None: ...

    async def flush_queue(self) -> None: ...


class Retriever(Protocol):
    async def get_request_response_payload(
        self,
        request_id: str,
        start_time_utc: datetime | None,
        end_time_utc: datetime | None,
    ) -> dict[str, JsonValue] | None: ...


class MockClient(Protocol):
    def mock_vertex_auth_methods(self) -> None: ...


class Probe(GCSBucketLogger):
    def config(self, dynamic: dict[str, JsonValue]) -> tuple[str, str]:
        key = self._get_config_key({"standard_callback_dynamic_params": dynamic})
        return key, self._sanitize_config_key(key)

    def date(self, value: datetime) -> str:
        return self._get_object_date_from_datetime(value)

    def drain_ids(self) -> list[str]:
        items = self._drain_queue_batch()
        return [item["payload"]["id"] for item in items]

    def remaining(self) -> int:
        return self.log_queue.qsize()

    def groups(self) -> JsonValue:
        grouped = self._group_items_by_config(self._drain_queue_batch())
        pairs = [
            [key, [item["payload"]["id"] for item in items]]
            for key, items in grouped.items()
        ]
        return JSON_VALUE.validate_python(pairs)

    def ndjson(self) -> tuple[list[JsonValue], bool]:
        text = self._combine_payloads_to_ndjson(self._drain_queue_batch())
        lines = [JSON_VALUE.validate_json(line) for line in text.split("\n")]
        return lines, text.endswith("\n")

    async def grouped_counts(self) -> tuple[int, int]:
        return await self._send_grouped_batch(self._drain_queue_batch(), "oracle")


class Recorder:
    def __init__(self, replies: list[Reply]) -> None:
        self.pending = list(replies)
        self.requests: list[dict[str, JsonValue]] = []

    def uploads(self) -> int:
        posts = [entry for entry in self.requests if entry["method"] == "POST"]
        return len(posts)

    def handle(self, request: httpx.Request) -> httpx.Response:
        text = request.content.decode()
        body: JsonValue = None
        url = str(request.url)
        if request.method == "POST":
            body = [JSON_VALUE.validate_json(line) for line in text.split("\n")]
            url = mask(url)
        entry: dict[str, JsonValue] = {
            "method": request.method,
            "url": url,
            "authorization": request.headers.get("authorization"),
            "body": body,
        }
        self.requests.append(entry)
        if self.pending:
            reply = self.pending.pop(0)
            return httpx.Response(reply["status"], text=reply["body"], request=request)
        return httpx.Response(200, text="{}", request=request)


def mask(url: str) -> str:
    masked = re.sub(r"\d{13}-[0-9a-f]{8}", "{batch}", url)
    masked = re.sub(r"\d{4}-\d{2}-\d{2}", "{date}", masked)
    return re.sub(r"[0-9a-f]{32}", "{uuid}", masked)


def event_kwargs(
    base: dict[str, JsonValue], step: Step, id_value: str
) -> dict[str, JsonValue]:
    response_id = step.get("response_id")
    payload: dict[str, JsonValue] = {
        **base,
        "id": id_value,
        "response": None if response_id is None else {"id": response_id},
    }
    kwargs: dict[str, JsonValue] = {"standard_logging_object": payload}
    dynamic = step.get("dynamic")
    if dynamic is not None:
        kwargs["standard_callback_dynamic_params"] = dynamic
    metadata = step.get("metadata")
    if metadata is not None:
        kwargs["litellm_params"] = {"metadata": metadata}
    return kwargs


async def queue_step(
    step: Step, input_value: Input, probe: Probe, sink: EventSink
) -> JsonValue:
    action = step["do"]
    if action in {"event", "failure"}:
        base = input_value.get("payload")
        if base is None:
            raise ValueError("an event step needs a payload")
        id_value = step.get("id")
        if id_value is None:
            raise ValueError("an event step needs an id")
        response_id = step.get("response_id")
        response: JsonValue = None if response_id is None else {"id": response_id}
        kwargs = event_kwargs(base, step, id_value)
        if action == "event":
            await sink.async_log_success_event(kwargs, response, 0.0, 0.0)
        else:
            await sink.async_log_failure_event(kwargs, response, 0.0, 0.0)
        return None
    if action == "settings":
        return JSON_VALUE.validate_python(
            [
                probe.batch_size,
                probe.flush_interval,
                probe.use_batched_logging,
                probe.log_queue.maxsize,
            ]
        )
    if action == "drain":
        return JSON_VALUE.validate_python(
            {"drained": probe.drain_ids(), "remaining": probe.remaining()}
        )
    if action == "group":
        return JSON_VALUE.validate_python({"groups": probe.groups()})
    lines, trailing = probe.ndjson()
    result: dict[str, JsonValue] = {"lines": lines, "trailing_newline": trailing}
    return result


async def describe_step(step: Step, input_value: Input, probe: Probe) -> JsonValue:
    action = step["do"]
    if action == "config":
        dynamic = input_value.get("dynamic")
        key, sanitized = probe.config({} if dynamic is None else dynamic)
        return JSON_VALUE.validate_python({"key": key, "sanitized": sanitized})
    if action == "retrieve":
        retriever: Retriever = probe
        request_id = input_value.get("request_id")
        start_time = input_value.get("start_time")
        if request_id is None or start_time is None:
            raise ValueError("a retrieve step needs request_id and start_time")
        start = datetime.fromtimestamp(start_time, tz=UTC)
        found = await retriever.get_request_response_payload(request_id, start, None)
        payload: JsonValue = None if found is None else JSON_VALUE.validate_python(found)
        return JSON_VALUE.validate_python({"payload": payload})
    timestamp = input_value.get("timestamp")
    if timestamp is None:
        raise ValueError("a date step needs timestamp")
    value = datetime.fromtimestamp(timestamp, tz=UTC)
    return JSON_VALUE.validate_python({"date": probe.date(value)})


async def send_step(step: Step, probe: Probe, recorder: Recorder) -> JsonValue:
    action = step["do"]
    background: Background = probe
    if action == "grouped":
        counts = await probe.grouped_counts()
        return JSON_VALUE.validate_python(list(counts))
    if action == "send":
        await background.async_send_batch()
        return JSON_VALUE.validate_python({"uploads": recorder.uploads()})
    if action == "flush":
        probe.last_flush_time = EPOCH_SECONDS
        await background.flush_queue()
        updated = probe.last_flush_time > EPOCH_SECONDS
        return JSON_VALUE.validate_python({"updated": updated, "uploads": recorder.uploads()})
    await asyncio.sleep(WAIT_SECONDS)
    return JSON_VALUE.validate_python({"uploads": recorder.uploads()})


async def perform(
    step: Step, input_value: Input, probe: Probe, sink: EventSink, recorder: Recorder
) -> JsonValue:
    if step["do"] in {"event", "failure", "settings", "drain", "group", "ndjson"}:
        return await queue_step(step, input_value, probe, sink)
    if step["do"] in {"config", "retrieve", "date"}:
        return await describe_step(step, input_value, probe)
    return await send_step(step, probe, recorder)


async def drive(input_value: Input) -> JsonValue:
    mock: MockClient = gcs_bucket_mock_client
    mock.mock_vertex_auth_methods()
    environment = input_value.get("environment")
    if environment is not None:
        os.environ.update(environment)
    probe = Probe(bucket_name=input_value.get("bucket"))
    replies = input_value.get("replies")
    recorder = Recorder([] if replies is None else replies)
    probe.async_httpx_client.client = httpx.AsyncClient(
        transport=httpx.MockTransport(recorder.handle)
    )
    results: list[JsonValue] = []
    for step in input_value["steps"]:
        results.append(await perform(step, input_value, probe, probe, recorder))
    return JSON_VALUE.validate_python({"steps": results, "requests": recorder.requests})


def run(input_value: Input) -> JsonValue:
    value: JsonValue = asyncio.run(drive(input_value))
    return value
