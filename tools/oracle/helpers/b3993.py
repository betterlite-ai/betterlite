import asyncio
from collections.abc import Callable
from datetime import UTC, datetime
from typing import Literal, TypeAlias, cast
from unittest.mock import patch

import botocore.auth
import httpx
import litellm
from botocore.credentials import Credentials
from litellm.integrations.adaptive_concurrency import AdaptiveConcurrencyLimiter, PutSample
from litellm.integrations.s3_v2 import S3Logger
from litellm.types.integrations.s3_v2 import s3BatchLoggingElement
from pydantic import BaseModel, Field, JsonValue

Json: TypeAlias = JsonValue


class Keys(BaseModel):
    access_key_id: str
    secret_access_key: str
    session_token: str | None = None


class Settings(BaseModel):
    bucket_name: str
    region_name: str
    endpoint_url: str | None = None
    use_virtual_hosted_style: bool = False
    server_side_encryption: str | None = None
    sse_kms_key_id: str | None = None
    drop_on_terminal_error: bool = True
    adaptive_concurrency: bool = False


class Element(BaseModel):
    s3_object_key: str
    s3_object_download_filename: str
    body: str | None = None
    content_type: Literal["application/json", "application/x-ndjson"] = "application/json"
    payload: dict[str, JsonValue] = Field(default_factory=dict)


class Reply(BaseModel):
    status: int
    body: str = ""


class Input(BaseModel):
    operation: Literal["put", "get"]
    settings: Settings
    credentials: Keys
    now: int
    element: Element | None = None
    object_key: str | None = None
    responses: list[Reply]


class Recorded(BaseModel):
    method: str
    url: str
    body: str | None
    headers: dict[str, str]


class ScriptedClient:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[Recorded] = []

    def respond(
        self, method: str, url: str, headers: dict[str, str], body: str | None
    ) -> httpx.Response:
        self.requests.append(
            Recorded(
                method=method,
                url=url,
                body=body,
                headers={name.lower(): value for name, value in headers.items()},
            )
        )
        reply = self.replies.pop(0)
        return httpx.Response(
            status_code=reply.status,
            content=reply.body.encode(),
            request=httpx.Request(method, url),
        )

    async def put(
        self, url: str, data: str | None = None, headers: dict[str, str] | None = None
    ) -> httpx.Response:
        return self.respond("PUT", url, {} if headers is None else headers, data)

    async def get(self, url: str, headers: dict[str, str] | None = None) -> httpx.Response:
        return self.respond("GET", url, {} if headers is None else headers, None)


class StaticCredentials:
    def __init__(self, credentials: Credentials) -> None:
        self.credentials = credentials

    def __call__(self, **_kwargs: object) -> Credentials:
        return self.credentials


class RecordingLimiter(AdaptiveConcurrencyLimiter):
    def __init__(self) -> None:
        super().__init__(1, 1, 8)
        self.samples: list[bool] = []

    def record(self, sample: PutSample) -> None:
        self.samples.append(sample.throttled)


def build_logger(
    input_value: Input, client: ScriptedClient, limiter: RecordingLimiter | None
) -> S3Logger:
    logger: S3Logger = object.__new__(S3Logger)
    initialize = cast(
        Callable[..., None],
        logger._init_s3_params,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    settings = input_value.settings
    initialize(
        params_source={},
        s3_bucket_name=settings.bucket_name,
        s3_region_name=settings.region_name,
        s3_endpoint_url=settings.endpoint_url,
        s3_use_virtual_hosted_style=settings.use_virtual_hosted_style,
        s3_server_side_encryption=settings.server_side_encryption,
        s3_sse_kms_key_id=settings.sse_kms_key_id,
        s3_drop_on_terminal_error=settings.drop_on_terminal_error,
        s3_adaptive_concurrency=settings.adaptive_concurrency,
    )
    credentials = input_value.credentials
    setattr(logger, "async_httpx_client", client)  # noqa: B010
    setattr(  # noqa: B010
        logger,
        "get_credentials",
        StaticCredentials(
            Credentials(
                credentials.access_key_id,
                credentials.secret_access_key,
                credentials.session_token,
            )
        ),
    )
    setattr(logger, "_upload_limiter", limiter)  # noqa: B010
    setattr(logger, "_flush_dropped", {})  # noqa: B010
    setattr(logger, "_flush_retries", 0)  # noqa: B010
    setattr(logger, "_requeued_count", 0)  # noqa: B010
    return logger


async def execute(input_value: Input) -> Json:
    limiter = RecordingLimiter() if input_value.settings.adaptive_concurrency else None
    client = ScriptedClient(input_value.responses)
    logger = build_logger(input_value, client, limiter)
    frozen = datetime.fromtimestamp(input_value.now, tz=UTC).replace(tzinfo=None)
    outcome: str | None = None
    samples: list[bool] | None = None
    object_value: Json = None
    with patch.object(botocore.auth, "get_current_datetime", lambda: frozen):
        if input_value.operation == "put":
            element_input = input_value.element
            if element_input is None:
                raise ValueError("a put call needs an element")
            element = s3BatchLoggingElement(
                payload=element_input.payload,
                s3_object_key=element_input.s3_object_key,
                s3_object_download_filename=element_input.s3_object_download_filename,
                body=element_input.body,
                content_type=element_input.content_type,
            )
            delivered = await logger.async_upload_data_to_s3(element)
            dropped = id(element) in logger._flush_dropped  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            outcome = "dropped" if dropped else ("delivered" if delivered else "retry")
            if limiter is not None:
                samples = limiter.samples
        else:
            if input_value.object_key is None:
                raise ValueError("a get call needs an object_key")
            object_value = cast(Json, await logger._download_object_from_s3(input_value.object_key))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    output: dict[str, Json] = {
        "requests": cast(Json, [record.model_dump() for record in client.requests]),
        "outcome": outcome,
        "samples": cast(Json, samples),
        "object": object_value,
    }
    return output


def run(input_value: dict[str, Json]) -> Json:
    _ = litellm
    return asyncio.run(execute(Input.model_validate(input_value)))
