import asyncio
import os
from collections.abc import Mapping
from typing import Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
from litellm.integrations.gcs_bucket import gcs_bucket_mock_client
from litellm.integrations.gcs_bucket.gcs_bucket_base import GCSBucketBase
from litellm.llms.vertex_ai.vertex_llm_base import VertexBase
from pydantic import JsonValue, TypeAdapter, ValidationError

JSON_VALUE = TypeAdapter(JsonValue)
PAYLOAD_KEYS = ("id", "trace_id", "call_type", "status", "total_tokens", "model")


class Input(TypedDict):
    action: Literal[
        "initialize",
        "async_headers",
        "sync_headers",
        "upload",
        "logging_config",
        "vertex_instance",
    ]
    environment: NotRequired[dict[str, str]]
    bucket: NotRequired[str]
    object_name: NotRequired[str]
    payload: NotRequired[str | dict[str, JsonValue]]
    dynamic: NotRequired[dict[str, JsonValue]]
    credentials: NotRequired[str]
    status: NotRequired[int]
    body: NotRequired[str]


class BucketView(Protocol):
    @property
    def is_mock_mode(self) -> bool: ...

    @property
    def path_service_account_json(self) -> str | None: ...

    @property
    def vertex_instances(self) -> Mapping[str, VertexBase]: ...

    async def download_gcs_object(self, object_name: str) -> bytes | None: ...

    async def get_or_create_vertex_instance(self, credentials: str | None) -> VertexBase: ...


class MockModule(Protocol):
    def mock_vertex_auth_methods(self) -> None: ...


class Probe(GCSBucketBase):
    async def upload(
        self, bucket: str, object_name: str, payload: str | dict[str, JsonValue]
    ) -> JsonValue:
        response = await self._log_json_data_on_gcs({}, bucket, object_name, payload)  # type: ignore[arg-type, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/gcs_bucket/gcs_bucket_base.py#L294
        return JSON_VALUE.validate_python(response)


class VertexFake:
    def __init__(self) -> None:
        self.ensure_calls: list[JsonValue] = []
        self.token_calls: list[JsonValue] = []

    def record(
        self, credentials: JsonValue, project_id: str | None, custom_llm_provider: str
    ) -> tuple[str, str]:
        entry: dict[str, JsonValue] = {
            "credentials": credentials,
            "project_id": project_id,
            "custom_llm_provider": custom_llm_provider,
        }
        self.ensure_calls.append(entry)
        return ("mock-gcs-token", "mock-project-id")

    def _ensure_access_token(
        self,
        credentials: JsonValue,
        project_id: str | None,
        custom_llm_provider: str,
    ) -> tuple[str, str]:
        return self.record(credentials, project_id, custom_llm_provider)

    async def _ensure_access_token_async(
        self,
        credentials: JsonValue,
        project_id: str | None,
        custom_llm_provider: str,
    ) -> tuple[str, str]:
        return self.record(credentials, project_id, custom_llm_provider)

    def _get_token_and_url(self, **arguments: JsonValue) -> tuple[str, str]:
        self.token_calls.append(dict(arguments))
        return ("mock-gcs-token", "https://original.example.com")


def parse_or_text(text: str) -> JsonValue:
    try:
        return JSON_VALUE.validate_json(text)
    except ValidationError:
        return text


def project_payload(body: JsonValue) -> JsonValue:
    match body:
        case {"id": _, "trace_id": _}:
            return {key: body[key] for key in PAYLOAD_KEYS}
        case _:
            return body


class Recorder:
    def __init__(self, status: int, body: str) -> None:
        self.status = status
        self.body = body
        self.requests: list[JsonValue] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        content = request.content.decode()
        headers: dict[str, str] = dict(request.headers)
        entry: dict[str, JsonValue] = {
            "method": request.method,
            "url": str(request.url),
            "authorization": headers.get("authorization"),
            "body": project_payload(parse_or_text(content)) if content else None,
        }
        self.requests.append(entry)
        return httpx.Response(self.status, text=self.body)

    def last(self) -> JsonValue:
        if self.requests:
            return self.requests[-1]
        return None


def scripted(base: GCSBucketBase, input_value: Input) -> Recorder:
    status = input_value.get("status")
    body = input_value.get("body")
    recorder = Recorder(200 if status is None else status, "" if body is None else body)
    transport = httpx.MockTransport(recorder.handle)
    base.async_httpx_client.client = httpx.AsyncClient(transport=transport)
    return recorder


def vertex_fake() -> VertexFake:
    fake = VertexFake()
    litellm.vertex_chat_completion = fake  # type: ignore[misc, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L320
    return fake


def header_result(fake: VertexFake, headers: JsonValue) -> JsonValue:
    result: dict[str, JsonValue] = {
        "headers": headers,
        "ensure": fake.ensure_calls[-1],
        "token": fake.token_calls[-1],
    }
    return result


async def download(base: BucketView, object_name: str) -> JsonValue:
    body = await base.download_gcs_object(object_name)
    if body is None:
        return None
    return parse_or_text(body.decode())


def cache_keys(base: BucketView) -> JsonValue:
    keys = sorted(base.vertex_instances.keys())
    return JSON_VALUE.validate_python(keys)


def initialize_case(input_value: Input) -> JsonValue:
    created = GCSBucketBase(bucket_name=input_value.get("bucket"))
    base: BucketView = created
    recorder = scripted(created, input_value)
    object_name = input_value.get("object_name")
    downloaded: JsonValue = None
    if object_name is not None:
        body = asyncio.run(download(base, object_name))
        downloaded = {"body": body, "request": recorder.last()}
    result: dict[str, JsonValue] = {
        "is_mock_mode": base.is_mock_mode,
        "path_service_account": base.path_service_account_json,
        "download": downloaded,
    }
    return result


def async_headers_case(input_value: Input) -> JsonValue:
    created = GCSBucketBase(bucket_name=input_value.get("bucket"))
    fake = vertex_fake()
    service_account = input_value.get("credentials")
    headers = asyncio.run(created.construct_request_headers(service_account_json=service_account))
    return header_result(fake, JSON_VALUE.validate_python(headers))


def sync_headers_case(input_value: Input) -> JsonValue:
    created = GCSBucketBase(bucket_name=input_value.get("bucket"))
    fake = vertex_fake()
    headers = created.sync_construct_request_headers()
    return header_result(fake, JSON_VALUE.validate_python(headers))


def upload_case(input_value: Input) -> JsonValue:
    bucket = input_value["bucket"]
    probe = Probe(bucket_name=bucket)
    recorder = scripted(probe, input_value)
    response = asyncio.run(probe.upload(bucket, input_value["object_name"], input_value["payload"]))
    result: dict[str, JsonValue] = {"response": response, "request": recorder.last()}
    return result


async def read_config(base: GCSBucketBase, dynamic: dict[str, JsonValue] | None) -> JsonValue:
    kwargs: dict[str, JsonValue] = {}
    if dynamic is not None:
        kwargs["standard_callback_dynamic_params"] = dynamic
    config = await base.get_gcs_logging_config(kwargs)
    result: dict[str, JsonValue] = {
        "bucket_name": config["bucket_name"],
        "path_service_account": config["path_service_account"],
    }
    return result


def logging_config_case(input_value: Input) -> JsonValue:
    created = GCSBucketBase(bucket_name=input_value.get("bucket"))
    base: BucketView = created
    resolved = asyncio.run(read_config(created, input_value.get("dynamic")))
    result: dict[str, JsonValue] = {"config": resolved, "cached_keys": cache_keys(base)}
    return result


async def compare_instances(base: BucketView, credentials: str | None) -> JsonValue:
    first = await base.get_or_create_vertex_instance(None)
    second = await base.get_or_create_vertex_instance(None)
    other = await base.get_or_create_vertex_instance(credentials)
    return {"same_instance": first is second, "distinct_credentials": first is not other}


def vertex_instance_case(input_value: Input) -> JsonValue:
    created = GCSBucketBase(bucket_name=input_value.get("bucket"))
    base: BucketView = created
    compared = asyncio.run(compare_instances(base, input_value.get("credentials")))
    result: dict[str, JsonValue] = {"instances": compared, "cached_keys": cache_keys(base)}
    return result


def install_auth(module: MockModule) -> None:
    methods = module.mock_vertex_auth_methods
    methods()


def run(input_value: Input) -> JsonValue:
    install_auth(gcs_bucket_mock_client)
    environment = input_value.get("environment")
    if environment is not None:
        for name, value in environment.items():
            os.environ[name] = value
    match input_value["action"]:
        case "initialize":
            return initialize_case(input_value)
        case "async_headers":
            return async_headers_case(input_value)
        case "sync_headers":
            return sync_headers_case(input_value)
        case "upload":
            return upload_case(input_value)
        case "logging_config":
            return logging_config_case(input_value)
        case "vertex_instance":
            return vertex_instance_case(input_value)
