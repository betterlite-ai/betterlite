import asyncio
from collections.abc import Callable
from functools import partial
from typing import Final, Literal, TypeAlias, TypedDict, TypeVar

import httpx
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.fal_ai.videos.transformation import FalAIVideoConfig, FalAIVideoError
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import VideoObject
from pydantic import JsonValue, TypeAdapter

BASE: Final = "https://queue.fal.run"
REQUEST_ID: Final = "req-1"
PROMPT: Final = "a cat"
BLOCKED_HEADERS: Final[frozenset[str]] = frozenset({"host", "content-length", "transfer-encoding"})
CREDENTIAL_HEADERS: Final[frozenset[str]] = frozenset(
    {"authorization", "proxy-authorization", "x-api-key", "api-key"}
)
ClientKind: TypeAlias = Literal["sync", "async"]
T = TypeVar("T")


class StatusCase(TypedDict):
    status_url: str
    headers: dict[str, str]
    body: str
    probe_status: int | None
    probe_body: str
    provider: str | None


class ContentCase(TypedDict):
    body: str
    status: int
    response_headers: dict[str, str]
    artifact_status: int | None
    artifact_body: str


class StatusReport(TypedDict):
    operation: Literal[
        "completed_terminal_error",
        "transient_probe_keeps_completed",
        "detail_locations",
        "recovered_request_identity",
    ]
    cases: list[StatusCase]


class ContentReport(TypedDict):
    operation: Literal["artifact_without_credentials", "missing_video_url"]
    cases: list[ContentCase]


class UnsupportedRequest(TypedDict):
    operation: Literal["unsupported_request"]


Input: TypeAlias = StatusReport | ContentReport | UnsupportedRequest


class VideoFields(TypedDict):
    id: str
    object: str
    status: str
    created_at: int | None
    completed_at: int | None
    expires_at: int | None
    error: dict[str, JsonValue] | None
    progress: int | None
    remixed_from_video_id: str | None
    seconds: str | None
    size: str | None
    model: str | None
    usage: dict[str, JsonValue] | None


VIDEO_FIELDS: TypeAdapter[VideoFields] = TypeAdapter(VideoFields)


class Script:
    def __init__(self, status: int | None, body: str) -> None:
        self.status = status
        self.body = body
        self.requests: list[httpx.Request] = []


def answer(script: Script, request: httpx.Request) -> httpx.Response:
    script.requests.append(request)
    if script.status is None:
        raise httpx.ConnectError("the scripted transport failed", request=request)
    return httpx.Response(status_code=script.status, content=script.body.encode(), request=request)


def configured(script: Script) -> FalAIVideoConfig:
    transport = httpx.MockTransport(partial(answer, script))
    return FalAIVideoConfig(
        sync_client_factory=partial(HTTPHandler, client=httpx.Client(transport=transport)),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L1376
        async_client_factory=partial(AsyncHTTPHandler, transport=transport),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L608
    )


def video_fields(video: VideoObject) -> JsonValue:
    fields = VIDEO_FIELDS.validate_json(video.model_dump_json())
    value: dict[str, JsonValue] = {}
    value["id"] = fields["id"]
    value["object"] = fields["object"]
    value["status"] = fields["status"]
    value["created_at"] = fields["created_at"]
    value["completed_at"] = fields["completed_at"]
    value["expires_at"] = fields["expires_at"]
    value["error"] = fields["error"]
    value["progress"] = fields["progress"]
    value["remixed_from_video_id"] = fields["remixed_from_video_id"]
    value["seconds"] = fields["seconds"]
    value["size"] = fields["size"]
    value["model"] = fields["model"]
    value["usage"] = fields["usage"]
    return value


def request_values(script: Script, names: frozenset[str]) -> list[JsonValue]:
    values: list[JsonValue] = []
    for request in script.requests:
        headers: dict[str, JsonValue] = {
            name: value
            for name, value in request.headers.multi_items()
            if name in names and name not in BLOCKED_HEADERS
        }
        recorded: JsonValue = {"url": str(request.url), "headers": headers}
        values.append(recorded)
    return values


def status_video(
    config: FalAIVideoConfig,
    kind: ClientKind,
    raw: httpx.Response,
    provider: str | None,
) -> VideoObject:
    if kind == "sync":
        return config.transform_video_status_retrieve_response(raw, None, provider)
    return asyncio.run(config.async_transform_video_status_retrieve_response(raw, None, provider))


def status_outcome(kind: ClientKind, case: StatusCase) -> JsonValue:
    script = Script(case["probe_status"], case["probe_body"])
    config = configured(script)
    raw = httpx.Response(
        status_code=200,
        content=case["body"].encode(),
        request=httpx.Request("GET", case["status_url"], headers=case["headers"]),
    )
    video = status_video(config, kind, raw, case["provider"])
    names = frozenset(name.lower() for name in case["headers"])
    answer_value: dict[str, JsonValue] = {
        "video": video_fields(video),
        "requests": request_values(script, names),
    }
    return answer_value


def content_bytes(config: FalAIVideoConfig, kind: ClientKind, raw: httpx.Response) -> bytes:
    if kind == "sync":
        return config.transform_video_content_response(raw, None)
    return asyncio.run(config.async_transform_video_content_response(raw, None))


def content_outcome(kind: ClientKind, case: ContentCase) -> JsonValue:
    script = Script(case["artifact_status"], case["artifact_body"])
    config = configured(script)
    raw = httpx.Response(
        status_code=case["status"],
        content=case["body"].encode(),
        headers=case["response_headers"],
        request=httpx.Request("GET", f"{BASE}/fal-ai/kling-video/requests/req-1"),
    )
    answer_value: dict[str, JsonValue] = {"body": None, "error": None, "requests": []}
    body: bytes | None = None
    try:
        body = content_bytes(config, kind, raw)
    except FalAIVideoError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/fal_ai/videos/transformation.py#L36
        answer_value["error"] = {
            "kind": "fal",
            "status": error.status_code,
            "message": error.message,
        }
    except httpx.HTTPStatusError as error:
        answer_value["error"] = {
            "kind": "artifact",
            "status": error.response.status_code,
            "message": None,
        }
    except httpx.TransportError:
        answer_value["error"] = {
            "kind": "transport",
            "status": None,
            "message": None,
        }
    except ValueError as error:
        answer_value["error"] = {
            "kind": "value",
            "status": None,
            "message": str(error),
        }
    answer_value["requests"] = request_values(script, CREDENTIAL_HEADERS)
    if body is not None:
        answer_value["body"] = body.decode("utf-8")
    return answer_value


def case_report(cases: list[T], outcome: Callable[[ClientKind, T], JsonValue]) -> JsonValue:
    outcomes: list[JsonValue] = []
    for case in cases:
        pair: dict[str, JsonValue] = {
            "sync": outcome("sync", case),
            "async": outcome("async", case),
        }
        outcomes.append(pair)
    answer_value: dict[str, JsonValue] = {"cases": outcomes}
    return answer_value


def attempted(name: str, call: Callable[[], T]) -> JsonValue:
    try:
        call()
    except NotImplementedError as error:
        failure: JsonValue = {"name": name, "message": str(error)}
        return failure
    success: JsonValue = {"name": name, "message": None}
    return success


def unsupported_report() -> JsonValue:
    config = FalAIVideoConfig()
    params = GenericLiteLLMParams()
    headers: dict[str, str] = {}
    remix = config.transform_video_remix_request
    listing = config.transform_video_list_request
    delete = config.transform_video_delete_request
    create_character = config.transform_video_create_character_request
    get_character = config.transform_video_get_character_request
    edit = config.transform_video_edit_request
    extension = config.transform_video_extension_request
    calls: list[JsonValue] = [
        attempted("remix", partial(remix, REQUEST_ID, PROMPT, BASE, params, headers)),
        attempted("list", partial(listing, BASE, params, headers)),
        attempted("delete", partial(delete, REQUEST_ID, BASE, params, headers)),
        attempted(
            "character_create",
            partial(create_character, "character", REQUEST_ID, BASE, params, headers),
        ),
        attempted(
            "character_get",
            partial(get_character, "character-1", REQUEST_ID, BASE, params, headers),
        ),
        attempted("edit", partial(edit, PROMPT, REQUEST_ID, BASE, params, headers)),
        attempted("extension", partial(extension, PROMPT, REQUEST_ID, "4", BASE, params, headers)),
    ]
    answer_value: dict[str, JsonValue] = {"operations": calls}
    return answer_value


def run(input_value: Input) -> JsonValue:
    match input_value["operation"]:
        case (
            "completed_terminal_error"
            | "transient_probe_keeps_completed"
            | "detail_locations"
            | "recovered_request_identity"
        ):
            return case_report(input_value["cases"], status_outcome)
        case "artifact_without_credentials" | "missing_video_url":
            return case_report(input_value["cases"], content_outcome)
        case "unsupported_request":
            return unsupported_report()
