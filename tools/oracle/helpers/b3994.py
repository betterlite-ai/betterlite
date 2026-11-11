import asyncio
import time
from collections.abc import Callable
from datetime import UTC, datetime, tzinfo
from functools import partial
from typing import NotRequired, TypeAlias, TypedDict
from uuid import UUID

import httpx
import litellm
from litellm.integrations import s3_v2
from litellm.integrations.adaptive_concurrency import AdaptiveConcurrencyLimiter, PutSample
from litellm.integrations.s3_v2 import S3Logger
from litellm.types.integrations.s3_v2 import s3BatchLoggingElement
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Keys(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: NotRequired[str | None]


class Element(TypedDict):
    s3_object_key: str
    s3_object_download_filename: str
    payload: dict[str, Json]
    body: NotRequired[str | None]
    content_type: NotRequired[str]
    retrying_since_offset: NotRequired[float | None]


class Reply(TypedDict):
    status: int
    body: NotRequired[str]


class ReplyRule(TypedDict):
    match: str
    responses: list[Reply]


class Step(TypedDict):
    enqueue: NotRequired[list[Element]]
    replies: NotRequired[list[ReplyRule]]


class Input(TypedDict):
    source: dict[str, Json]
    credentials: Keys
    now: int
    cold_storage_logger: NotRequired[str | None]
    steps: list[Step]


class FixedCredentials:
    def __init__(self, keys: Keys) -> None:
        self.access_key = keys["access_key_id"]
        self.secret_key = keys["secret_access_key"]
        self.token = keys.get("session_token")


class CredentialProvider:
    def __init__(self, keys: Keys) -> None:
        self.credentials = FixedCredentials(keys)

    def __call__(self, **_kwargs: str | None) -> FixedCredentials:
        return self.credentials


class ScriptedClient:
    def __init__(self) -> None:
        self.rules: list[ReplyRule] = []
        self.served: dict[str, int] = {}
        self.requests: list[tuple[str, str]] = []
        self.in_flight = 0
        self.peak = 0

    def reply_for(self, url: str) -> Reply:
        rule = next((item for item in self.rules if item["match"] in url), None)
        if rule is None:
            return {"status": 200}
        try:
            index = self.served[rule["match"]]
        except KeyError:
            index = 0
        self.served[rule["match"]] = index + 1
        if index < len(rule["responses"]):
            return rule["responses"][index]
        return {"status": 200}

    async def put(
        self, url: str, data: str | None = None, headers: dict[str, str] | None = None
    ) -> httpx.Response:
        resolved = headers if headers is not None else {}
        self.requests.append((url, data if data is not None else ""))
        self.in_flight += 1
        self.peak = max(self.peak, self.in_flight)
        reply = self.reply_for(url)
        for _ in range(2):
            await asyncio.sleep(0)
        self.in_flight -= 1
        try:
            body = reply["body"]
        except KeyError:
            body = ""
        return httpx.Response(
            status_code=reply["status"],
            content=body.encode(),
            request=httpx.Request("PUT", url, headers=resolved),
        )


class RecordingLimiter(AdaptiveConcurrencyLimiter):
    def __init__(self, window: int, ceiling: int) -> None:
        super().__init__(window, window, ceiling)
        self.samples: list[bool] = []

    @override
    def record(self, sample: PutSample) -> None:
        self.samples.append(sample.throttled)


class FrozenDateTime:
    def __init__(self, instant: datetime) -> None:
        self.instant = instant

    def now(self, tz: tzinfo | None = None) -> datetime:
        if tz is None:
            return self.instant.replace(tzinfo=None)
        return self.instant


def build_logger(
    source: dict[str, Json], keys: Keys, client: ScriptedClient
) -> tuple[S3Logger, RecordingLimiter | None]:
    logger = object.__new__(S3Logger)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L118
    logger._init_s3_params(params_source=source)  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L256
    logger.async_httpx_client = client  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L227
    logger.get_credentials = CredentialProvider(keys)
    logger.log_queue = []
    logger._requeued_count = 0
    logger._flush_dropped = {}
    logger._flush_retries = 0
    logger.flush_lock = asyncio.Lock()
    recording = None
    if logger.s3_adaptive_concurrency:
        recording = RecordingLimiter(
            logger.s3_max_concurrent_uploads,
            max(logger.s3_max_concurrent_uploads, logger.s3_max_adaptive_concurrency),
        )
        logger._upload_limiter = recording
    else:
        logger._upload_limiter = asyncio.Semaphore(logger.s3_max_concurrent_uploads)
    return logger, recording


async def execute(input_value: Input) -> dict[str, Json]:
    client = ScriptedClient()
    logger, recording = build_logger(input_value["source"], input_value["credentials"], client)
    fixed = datetime.fromtimestamp(input_value["now"], tz=UTC)
    step_results: list[Json] = []
    datetime_previous: type[datetime] = s3_v2.datetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L765
    uuid_previous: Callable[[], UUID] = s3_v2.uuid4  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L765
    cold_previous = litellm.cold_storage_custom_logger
    try:
        s3_v2.datetime = FrozenDateTime(fixed)  # type: ignore[attr-defined, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L765
        s3_v2.uuid4 = partial(UUID, int=0)  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L765
        cold = input_value.get("cold_storage_logger")
        if cold == "s3_v2":
            litellm.cold_storage_custom_logger = "s3_v2"
        else:
            litellm.cold_storage_custom_logger = None
        for step in input_value["steps"]:
            try:
                client.rules = step["replies"]
            except KeyError:
                client.rules = []
            client.served = {}
            client.requests = []
            client.in_flight = 0
            client.peak = 0
            try:
                enqueue = step["enqueue"]
            except KeyError:
                enqueue = []
            for item in enqueue:
                offset = item.get("retrying_since_offset")
                try:
                    content_type = item["content_type"]
                except KeyError:
                    content_type = "application/json"
                try:
                    body = item["body"]
                except KeyError:
                    body = None
                logger.log_queue.append(
                    s3BatchLoggingElement(
                        payload=item["payload"],
                        s3_object_key=item["s3_object_key"],
                        s3_object_download_filename=item["s3_object_download_filename"],
                        body=body,
                        content_type=content_type,
                        retrying_since=None if offset is None else time.monotonic() + offset,
                    )
                )
            await logger.flush_queue()  # type: ignore[no-untyped-call, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_batch_logger.py#L54
            queued: list[Json] = [
                {
                    "key": element.s3_object_key,
                    "retrying_since": element.retrying_since is not None,
                    "body": element.body,
                }
                for element in logger.log_queue
            ]
            samples: list[Json] | None = None
            if recording is not None:
                samples = list(recording.samples)
            requests: list[Json] = [
                {"url": url, "body": body} for url, body in sorted(client.requests)
            ]
            limit = (
                recording.limit
                if recording is not None
                else logger.s3_max_concurrent_uploads
            )
            step_results.append(
                {
                    "requests": requests,
                    "queue": queued,
                    "requeued_count": logger._requeued_count,  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L246
                    "samples": samples,
                    "limit": limit,
                    "peak": client.peak,
                }
            )
    finally:
        s3_v2.datetime = datetime_previous  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L765
        s3_v2.uuid4 = uuid_previous  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L765
        litellm.cold_storage_custom_logger = cold_previous
    return {"steps": step_results}


def run(input_value: Input) -> Json:
    coroutine = execute(input_value)
    return asyncio.run(coroutine)
