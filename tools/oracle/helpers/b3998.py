import asyncio
import os
import re
from asyncio import Queue
from datetime import UTC, datetime
from typing import Literal, NotRequired, Protocol, TypedDict, TypeVar

import httpx
from litellm.integrations.gcs_bucket import gcs_bucket_mock_client
from litellm.integrations.gcs_bucket.gcs_bucket import GCSBucketLogger
from litellm.proxy import proxy_server
from litellm.types.integrations.gcs_bucket import GCSLogQueueItem
from pydantic import JsonValue, TypeAdapter

JSON_VALUE = TypeAdapter(JsonValue)
WAIT_SECONDS = 1.5
# LiteLLM gates GCS logging on its paid tier, which betterlite does not have
proxy_server.premium_user = True

Item = TypeVar("Item")
QueueAction = Literal["event", "failure", "settings", "drain", "group", "ndjson"]
DescribeAction = Literal["config", "retrieve", "date"]
SendAction = Literal["grouped", "send", "flush", "wait"]


class Reply(TypedDict):
    status: int
    body: str


class Step(TypedDict):
    do: QueueAction | DescribeAction | SendAction
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


class MockClient(Protocol):
    def mock_vertex_auth_methods(self) -> None: ...


class PayloadId(TypedDict):
    id: str


class QueueEntry(TypedDict):
    payload: PayloadId


QUEUE_ENTRIES = TypeAdapter(list[QueueEntry])


# the logger's queue items carry Any, so the members that touch them are typed here
class Bridge(Protocol[Item]):
    batch_size: int
    flush_interval: float
    use_batched_logging: bool
    log_queue: Queue[Item]

    def _drain_queue_batch(self) -> list[Item]: ...
    def _group_items_by_config(self, items: list[Item]) -> dict[str, list[Item]]: ...
    def _combine_payloads_to_ndjson(self, items: list[Item]) -> str: ...
    async def _send_grouped_batch(
        self, items: list[Item], config_key: str
    ) -> tuple[int, int]: ...
    def _get_config_key(self, kwargs: dict[str, JsonValue]) -> str: ...
    def _sanitize_config_key(self, config_key: str) -> str: ...
    def _get_object_date_from_datetime(self, datetime_obj: datetime) -> str: ...
    async def get_request_response_payload(
        self,
        request_id: str,
        start_time_utc: datetime | None,
        end_time_utc: datetime | None,
    ) -> dict[str, JsonValue] | None: ...
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
    async def async_send_batch(self) -> None: ...
    async def flush_queue(self) -> None: ...

    async def oracle_event(
        self, kwargs: dict[str, JsonValue], response: JsonValue, *, failed: bool
    ) -> None:
        if failed:
            await self.async_log_failure_event(kwargs, response, 0.0, 0.0)
        else:
            await self.async_log_success_event(kwargs, response, 0.0, 0.0)

    async def oracle_send(self, *, flush: bool) -> None:
        if flush:
            await self.flush_queue()
        else:
            await self.async_send_batch()

    def oracle_settings(self) -> JsonValue:
        return [
            self.batch_size,
            self.flush_interval,
            self.use_batched_logging,
            self.log_queue.maxsize,
        ]

    def oracle_ids(self, items: list[Item]) -> list[JsonValue]:
        entries = QUEUE_ENTRIES.validate_python(items)
        return [entry["payload"]["id"] for entry in entries]

    def oracle_drain(self) -> JsonValue:
        ids = self.oracle_ids(self._drain_queue_batch())
        return {"drained": ids, "remaining": self.log_queue.qsize()}

    def oracle_groups(self) -> JsonValue:
        grouped = self._group_items_by_config(self._drain_queue_batch())
        pairs: list[JsonValue] = [
            [key, self.oracle_ids(items)] for key, items in grouped.items()
        ]
        return {"groups": pairs}

    def oracle_ndjson(self) -> JsonValue:
        text = self._combine_payloads_to_ndjson(self._drain_queue_batch())
        lines = [JSON_VALUE.validate_json(line) for line in text.split("\n")]
        return {"lines": lines, "trailing_newline": text.endswith("\n")}

    async def oracle_grouped(self) -> JsonValue:
        sent, errors = await self._send_grouped_batch(self._drain_queue_batch(), "oracle")
        return [sent, errors]

    def oracle_config(self, dynamic: dict[str, JsonValue]) -> JsonValue:
        key = self._get_config_key({"standard_callback_dynamic_params": dynamic})
        return {"key": key, "sanitized": self._sanitize_config_key(key)}

    async def oracle_retrieve(self, request_id: str, start: datetime) -> JsonValue:
        return {"payload": await self.get_request_response_payload(request_id, start, None)}

    def oracle_date(self, value: datetime) -> JsonValue:
        return {"date": self._get_object_date_from_datetime(value)}


class Probe(GCSBucketLogger, Bridge[GCSLogQueueItem]):
    pass


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
            url = re.sub(r"\d{13}-[0-9a-f]{8}", "{batch}", url)
            url = re.sub(r"\d{4}-\d{2}-\d{2}", "{date}", url)
            url = re.sub(r"[0-9a-f]{32}", "{uuid}", url)
        authorization = request.headers.get_list("authorization")
        entry: dict[str, JsonValue] = {
            "method": request.method,
            "url": url,
            "authorization": authorization[0] if authorization else None,
            "body": body,
        }
        self.requests.append(entry)
        if self.pending:
            reply = self.pending.pop(0)
            return httpx.Response(reply["status"], text=reply["body"], request=request)
        return httpx.Response(200, text="{}", request=request)


def event_kwargs(
    base: dict[str, JsonValue], step: Step, id_value: str, response: JsonValue
) -> dict[str, JsonValue]:
    payload = {**base, "id": id_value, "response": response}
    kwargs: dict[str, JsonValue] = {"standard_logging_object": payload}
    if "dynamic" in step:
        kwargs["standard_callback_dynamic_params"] = step["dynamic"]
    if "metadata" in step:
        kwargs["litellm_params"] = {"metadata": step["metadata"]}
    return kwargs


async def queue_step(step: Step, input_value: Input, probe: Probe) -> JsonValue:
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
        kwargs = event_kwargs(base, step, id_value, response)
        await probe.oracle_event(kwargs, response, failed=action == "failure")
        return None
    if action == "settings":
        return probe.oracle_settings()
    if action == "drain":
        return probe.oracle_drain()
    if action == "group":
        return probe.oracle_groups()
    return probe.oracle_ndjson()


async def describe_step(step: Step, input_value: Input, probe: Probe) -> JsonValue:
    action = step["do"]
    if action == "config":
        dynamic = input_value.get("dynamic")
        return probe.oracle_config({} if dynamic is None else dynamic)
    if action == "retrieve":
        request_id = input_value.get("request_id")
        start_time = input_value.get("start_time")
        if request_id is None or start_time is None:
            raise ValueError("a retrieve step needs request_id and start_time")
        start = datetime.fromtimestamp(start_time, tz=UTC)
        return await probe.oracle_retrieve(request_id, start)
    timestamp = input_value.get("timestamp")
    if timestamp is None:
        raise ValueError("a date step needs timestamp")
    return probe.oracle_date(datetime.fromtimestamp(timestamp, tz=UTC))


async def send_step(step: Step, probe: Probe, recorder: Recorder) -> JsonValue:
    action = step["do"]
    if action == "grouped":
        return await probe.oracle_grouped()
    report: dict[str, JsonValue] = {}
    if action == "send":
        await probe.oracle_send(flush=False)
    elif action == "flush":
        probe.last_flush_time = 0.0
        await probe.oracle_send(flush=True)
        report["updated"] = probe.last_flush_time > 0.0
    else:
        await asyncio.sleep(WAIT_SECONDS)
    report["uploads"] = recorder.uploads()
    return report


async def perform(
    step: Step, input_value: Input, probe: Probe, recorder: Recorder
) -> JsonValue:
    if step["do"] in {"event", "failure", "settings", "drain", "group", "ndjson"}:
        return await queue_step(step, input_value, probe)
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
    results = [
        await perform(step, input_value, probe, recorder)
        for step in input_value["steps"]
    ]
    requests: list[JsonValue] = [*recorder.requests]
    return {"steps": results, "requests": requests}


def run(input_value: Input) -> JsonValue:
    value: JsonValue = asyncio.run(drive(input_value))
    return value
