import asyncio
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from functools import partial
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/auth.py#L33
import httpx
from litellm.integrations.adaptive_concurrency import AdaptiveConcurrencyLimiter, PutSample
from litellm.integrations.s3_v2 import S3Logger
from litellm.types.integrations.s3_v2 import s3BatchLoggingElement
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Keys(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: NotRequired[str | None]


class Settings(TypedDict):
    bucket_name: str
    region_name: str
    endpoint_url: NotRequired[str | None]
    use_virtual_hosted_style: NotRequired[bool]
    server_side_encryption: NotRequired[str | None]
    sse_kms_key_id: NotRequired[str | None]
    drop_on_terminal_error: NotRequired[bool]
    adaptive_concurrency: NotRequired[bool]


class Element(TypedDict):
    s3_object_key: str
    s3_object_download_filename: str
    body: NotRequired[str | None]
    content_type: NotRequired[str]
    payload: NotRequired[dict[str, Json]]


class Reply(TypedDict):
    status: int
    body: NotRequired[str]


class Input(TypedDict):
    operation: Literal["put", "get"]
    settings: Settings
    credentials: Keys
    now: int
    element: NotRequired[Element]
    object_key: NotRequired[str]
    responses: list[Reply]


class FixedCredentials:
    access_key: str
    secret_key: str
    token: str | None

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
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[Json] = []

    def respond(
        self, method: str, url: str, headers: dict[str, str], body: str | None
    ) -> httpx.Response:
        record: dict[str, Json] = {
            "method": method,
            "url": url,
            "body": body,
            "headers": {name.lower(): value for name, value in headers.items()},
        }
        self.requests.append(record)
        reply = self.replies.pop(0)
        response_body = reply.get("body")
        return httpx.Response(
            status_code=reply["status"],
            content=(response_body if response_body is not None else "").encode(),
            request=httpx.Request(method, url),
        )

    async def put(
        self, url: str, data: str | None = None, headers: dict[str, str] | None = None
    ) -> httpx.Response:
        resolved = headers if headers is not None else {}
        return self.respond("PUT", url, resolved, data)

    async def get(self, url: str, headers: dict[str, str] | None = None) -> httpx.Response:
        resolved = headers if headers is not None else {}
        return self.respond("GET", url, resolved, None)


class RecordingLimiter(AdaptiveConcurrencyLimiter):
    def __init__(self) -> None:
        super().__init__(1, 1, 8)
        self.samples: list[Json] = []

    @override
    def record(self, sample: PutSample) -> None:
        self.samples.append(sample.throttled)


def frozen_datetime(epoch: int) -> datetime:
    aware = datetime.fromtimestamp(epoch, tz=UTC)
    return aware.replace(tzinfo=None)


def freeze_auth_clock(epoch: int) -> None:
    botocore.auth.get_current_datetime = partial(frozen_datetime, epoch)  # type: ignore[misc]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/auth.py#L33


def build_logger(
    settings: Settings, keys: Keys, client: ScriptedClient, limiter: RecordingLimiter | None
) -> S3Logger:
    logger = object.__new__(S3Logger)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L118
    empty_source: dict[str, str] = {}
    virtual_hosted = settings.get("use_virtual_hosted_style")
    drop_terminal = settings.get("drop_on_terminal_error")
    adaptive = settings.get("adaptive_concurrency")
    logger._init_s3_params(  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L256
        params_source=empty_source,
        s3_bucket_name=settings["bucket_name"],
        s3_region_name=settings["region_name"],
        s3_endpoint_url=settings.get("endpoint_url"),
        s3_use_virtual_hosted_style=False if virtual_hosted is None else virtual_hosted,
        s3_server_side_encryption=settings.get("server_side_encryption"),
        s3_sse_kms_key_id=settings.get("sse_kms_key_id"),
        s3_drop_on_terminal_error=True if drop_terminal is None else drop_terminal,
        s3_adaptive_concurrency=False if adaptive is None else adaptive,
    )
    logger.async_httpx_client = client  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L227
    logger.get_credentials = CredentialProvider(keys)  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L346
    if limiter is not None:
        logger._upload_limiter = limiter  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L213
    logger._flush_dropped = {}  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L247
    return logger


async def execute(input_value: Input) -> dict[str, Json]:
    settings = input_value["settings"]
    adaptive = settings.get("adaptive_concurrency")
    limiter = RecordingLimiter() if adaptive is True else None
    client = ScriptedClient(input_value["responses"])
    logger = build_logger(settings, input_value["credentials"], client, limiter)
    freeze_auth_clock(input_value["now"])
    outcome: str | None = None
    samples: list[Json] | None = None
    object_value: dict[str, Json] | None = None
    if input_value["operation"] == "put":
        element_input = input_value.get("element")
        if element_input is None:
            raise ValueError("a put call needs an element")
        payload_value = element_input.get("payload")
        content_type_value = element_input.get("content_type")
        payload: dict[str, Json] = {} if payload_value is None else payload_value
        element = s3BatchLoggingElement(
            payload=payload,
            s3_object_key=element_input["s3_object_key"],
            s3_object_download_filename=element_input["s3_object_download_filename"],
            body=element_input.get("body"),
            content_type="application/json" if content_type_value is None else content_type_value,
        )
        delivered = await logger.async_upload_data_to_s3(element)
        dropped = id(element) in logger._flush_dropped  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L247
        outcome = "dropped" if dropped else ("delivered" if delivered else "retry")
        if limiter is not None:
            samples = limiter.samples
    else:
        object_key = input_value.get("object_key")
        if object_key is None:
            raise ValueError("a get call needs an object_key")
        download = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L923
            Callable[[str], Awaitable[dict[str, Json] | None]],
            logger._download_object_from_s3,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L923
        )
        object_value = await download(object_key)
    return {
        "requests": client.requests,
        "outcome": outcome,
        "samples": samples,
        "object": object_value,
    }


def run(input_value: Input) -> Json:
    coroutine = execute(input_value)
    return asyncio.run(coroutine)
