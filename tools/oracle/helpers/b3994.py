import asyncio
import json
import re
import time
from datetime import UTC, datetime, tzinfo
from typing import NotRequired, TypeAlias, TypedDict

import httpx
import litellm
from litellm.integrations import s3_v2
from litellm.integrations.adaptive_concurrency import AdaptiveConcurrencyLimiter
from litellm.integrations.s3_v2 import S3Logger
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.types.integrations.s3_v2 import s3BatchLoggingElement

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Element(TypedDict):
    s3_object_key: str
    s3_object_download_filename: str
    payload: dict[str, Json]
    retrying_since_offset: NotRequired[float | None]


class Reply(TypedDict):
    status: int
    body: NotRequired[str]


class Rule(TypedDict):
    match: str
    responses: list[Reply]


class Step(TypedDict):
    enqueue: NotRequired[list[Element]]
    replies: NotRequired[list[Rule]]


class Input(TypedDict):
    source: dict[str, Json]
    now: int
    cold_storage_logger: NotRequired[str | None]
    steps: list[Step]


class Client:
    def __init__(self) -> None:
        self.rules: list[Rule] = []
        self.served: dict[str, int] = {}
        self.requests: list[tuple[str, str]] = []

    def reply(self, url: str) -> Reply:
        rule = next((item for item in self.rules if item["match"] in url), None)
        if rule is None:
            return {"status": 200}
        try:
            index = self.served[rule["match"]]
        except KeyError:
            index = 0
        self.served[rule["match"]] = index + 1
        responses = rule["responses"]
        return responses[index] if index < len(responses) else {"status": 200}

    async def put(self, url: str, data: str | None = None, **_kwargs: Json) -> httpx.Response:
        self.requests.append((url, data if data is not None else ""))
        reply = self.reply(url)
        for _ in range(2):
            await asyncio.sleep(0)
        try:
            body = reply["body"]
        except KeyError:
            body = ""
        return httpx.Response(
            status_code=reply["status"],
            content=body.encode(),
            request=httpx.Request("PUT", url),
        )


class Frozen:
    def __init__(self, instant: datetime) -> None:
        self.instant = instant

    def now(self, tz: tzinfo | None = None) -> datetime:
        if tz is None:
            return self.instant.replace(tzinfo=None)
        return self.instant


def build_logger(
    source: dict[str, Json], client: Client
) -> tuple[S3Logger, AdaptiveConcurrencyLimiter | None]:
    logger = object.__new__(S3Logger)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L118
    logger._init_s3_params(params_source=source)  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L256
    logger.async_httpx_client = client  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L227
    logger.log_queue = []
    logger.max_queue_size = logger.s3_max_queue_size
    BaseAWSLLM.__init__(logger)
    logger._requeued_count = 0
    logger._flush_dropped = {}
    logger._flush_retries = 0
    logger.flush_lock = asyncio.Lock()
    limiter = None
    if logger.s3_adaptive_concurrency:
        limiter = AdaptiveConcurrencyLimiter(
            logger.s3_max_concurrent_uploads,
            logger.s3_max_concurrent_uploads,
            max(logger.s3_max_concurrent_uploads, logger.s3_max_adaptive_concurrency),
        )
        logger._upload_limiter = limiter
    else:
        logger._upload_limiter = asyncio.Semaphore(logger.s3_max_concurrent_uploads)
    return logger, limiter


def body_lines(body: str) -> list[Json]:
    lines: list[Json] = []
    for line in body.split("\n"):
        try:
            value: Json = json.loads(line)
        except json.JSONDecodeError:
            value = line
        lines.append(value)
    return lines


def element(item: Element) -> s3BatchLoggingElement:
    offset = item.get("retrying_since_offset")
    return s3BatchLoggingElement(
        payload=item["payload"],
        s3_object_key=item["s3_object_key"],
        s3_object_download_filename=item["s3_object_download_filename"],
        retrying_since=None if offset is None else time.monotonic() + offset,
    )


async def execute(input_value: Input) -> dict[str, Json]:
    client = Client()
    logger, limiter = build_logger(input_value["source"], client)
    fixed = datetime.fromtimestamp(input_value["now"], tz=UTC)
    step_results: list[Json] = []
    datetime_previous: type[datetime] = s3_v2.datetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L765
    cold_previous = litellm.cold_storage_custom_logger
    try:
        s3_v2.datetime = Frozen(fixed)  # type: ignore[attr-defined, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L765
        if input_value.get("cold_storage_logger") == "s3_v2":
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
            try:
                queued = step["enqueue"]
            except KeyError:
                queued = []
            for item in queued:
                logger.log_queue.append(element(item))
            await logger.flush_queue()  # type: ignore[no-untyped-call, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_batch_logger.py#L54
            remaining: list[Json] = [
                {
                    "key": item.s3_object_key,
                    "retrying_since": item.retrying_since is not None,
                    "body": None if item.body is None else body_lines(item.body),
                }
                for item in logger.log_queue
            ]
            fallback = logger.s3_max_concurrent_uploads
            requests: list[Json] = [
                {"url": re.sub(r"[0-9a-f]{32}", "ID", url), "body": body_lines(body)}
                for url, body in sorted(client.requests)
            ]
            step_results.append(
                {
                    "requests": requests,
                    "queue": remaining,
                    "requeued_count": logger._requeued_count,  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L246
                    "limit": (fallback if limiter is None else limiter.limit),
                }
            )
    finally:
        s3_v2.datetime = datetime_previous  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L765
        litellm.cold_storage_custom_logger = cold_previous
    return {"steps": step_results}


def run(input_value: Input) -> Json:
    coroutine = execute(input_value)
    return asyncio.run(coroutine)
