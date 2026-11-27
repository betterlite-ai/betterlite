import asyncio
import copy
import importlib
import inspect
import json
import threading
from collections.abc import Awaitable
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import ClassVar, Final, NamedTuple, Protocol, TypeVar, runtime_checkable
from urllib.parse import urlsplit

import pytest
from batch_server import parameter_rows
from pydantic import JsonValue, TypeAdapter
from test_videos_calls import Part, sent_parts
from typing_extensions import TypedDict

from betterlite._completion.errors import ERROR_PAYLOAD, UnsupportedCallArgument
from betterlite._endpoints.payload import CallValue
from betterlite._native import NativeVideoEditError
from betterlite.exceptions import AuthenticationError, BadRequestError
from betterlite.types.videos.main import VideoCreateOptionalRequestParams as CreateParams
from betterlite.videos._utils import (
    VideoProviderConfig,
    map_video_optional_params,
    requested_optional_params,
)

HOST: Final = "127.0.0.1"
OK: Final = 200
BAD_REQUEST: Final = 400
UNAUTHORIZED: Final = 401
NOT_FOUND: Final = 404
GOOD_KEY: Final = "sk-test"
BAD_KEY: Final = "bad-key"
PROVIDER: Final = "openai"
REFUSED_CHARACTER_PROVIDER: Final = "runwayml"
REFUSED_EDIT_PROVIDER: Final = "gemini"
MODEL: Final = "sora-2"
CHARACTER_ID: Final = "char-test"
CHARACTER_NAME: Final = "spot"
VIDEO_ID: Final = "vid-test"
PROMPT: Final = "a kite over the bay"
SECONDS: Final = "8"
NAME_FIELD: Final = "name"
VIDEO_FIELD: Final = "video"
PROMPT_FIELD: Final = "prompt"
STYLE: Final = "noir"
CREATED_AT: Final = 1710000000
API_PREFIX: Final = "/v1"
VIDEOS_PATH: Final = f"{API_PREFIX}/videos"
CHARACTERS_PATH: Final = f"{VIDEOS_PATH}/characters"
CHARACTER_PATH: Final = f"{CHARACTERS_PATH}/{CHARACTER_ID}"
EDITS_PATH: Final = f"{VIDEOS_PATH}/edits"
EXTENSIONS_PATH: Final = f"{VIDEOS_PATH}/extensions"
JSON_TYPE: Final = "application/json"
MEDIA_TYPE: Final = "video/mp4"
MEDIA_FILENAME: Final = "input_video.mp4"
CLIP_NAME: Final = "clip.mp4"
MEDIA_BYTES: Final = b"\x00\x00\x00\x18ftypmp42" + b"\x00" * 24
AUTH_TEXT: Final = "Incorrect API key provided: bad-key"
CRLF: Final = b"\r\n"
HEADER_END: Final = b"\r\n\r\n"
TYPED_NAMES: Final[tuple[str, ...]] = (
    "VideoCreateOptionalRequestParams",
    "VideoCreateRequestParams",
    "VideoEditRequestParams",
    "VideoExtensionRequestParams",
)
AUTH_BODY: Final[dict[str, JsonValue]] = {
    "error": {"message": AUTH_TEXT, "type": "invalid_request_error"},
}
MISSING_BODY: Final[dict[str, JsonValue]] = {
    "error": {"message": "no such target", "type": "not_found_error", "code": "not_found"},
}
CHARACTER_RECORD: Final[dict[str, JsonValue]] = {
    "id": CHARACTER_ID,
    "object": "character",
    "created_at": CREATED_AT,
    "name": CHARACTER_NAME,
}
MOCKED_CHARACTER: Final[dict[str, JsonValue]] = {
    "id": "char-mocked",
    "object": "character",
    "created_at": CREATED_AT,
    "name": CHARACTER_NAME,
}
EDITED_RECORD: Final[dict[str, JsonValue]] = {
    "id": "vid-edited",
    "object": "video",
    "status": "completed",
    "created_at": CREATED_AT,
    "seconds": SECONDS,
}
EXTENDED_RECORD: Final[dict[str, JsonValue]] = {
    "id": "vid-extended",
    "object": "video",
    "status": "completed",
    "created_at": CREATED_AT,
    "completed_at": CREATED_AT + 60,
    "seconds": "12",
}
JSON_RECORD: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
REQUESTED_PARAMS: Final[dict[str, JsonValue]] = {
    "prompt": PROMPT,
    "model": MODEL,
    "seconds": SECONDS,
    "timeout": 600,
    "api_key": GOOD_KEY,
    "custom_llm_provider": PROVIDER,
    "extra_body": {"style": STYLE, "skip": None},
    "kwargs": {
        "extra_body": {"style": "loose", "second": "2"},
        "user": "user-1",
        "unused": None,
    },
}


class RecordedCall(TypedDict):
    method: str
    path: str
    headers: dict[str, str]
    body: bytes


@runtime_checkable
class DumpedObject(Protocol):
    def model_dump_json(self) -> str: ...


class EditCall(Protocol):
    def __call__(self, **kwargs: CallValue) -> DumpedObject: ...


class AsyncEditCall(Protocol):
    def __call__(self, **kwargs: CallValue) -> Awaitable[DumpedObject]: ...


class EditModule(Protocol):
    def __getattr__(self, name: str) -> EditCall: ...


class AsyncEditModule(Protocol):
    def __getattr__(self, name: str) -> AsyncEditCall: ...


class NamesModule(Protocol):
    def __getattr__(self, name: str) -> list[str]: ...


class ErrorClasses(Protocol):
    def __getattr__(self, name: str) -> type[BaseException]: ...


class OptionalCall(Protocol):
    def __call__(
        self,
        model: str,
        provider_config: VideoProviderConfig,
        optional_params: CreateParams,
    ) -> dict[str, JsonValue]: ...


class OptionalClass(Protocol):
    def __getattr__(self, name: str) -> OptionalCall: ...


class OptionalModule(Protocol):
    def __getattr__(self, name: str) -> OptionalClass: ...


class RequestedCall(Protocol):
    def __call__(self, params: dict[str, JsonValue]) -> dict[str, JsonValue]: ...


class RequestedClass(Protocol):
    def __getattr__(self, name: str) -> RequestedCall: ...


class RequestedModule(Protocol):
    def __getattr__(self, name: str) -> RequestedClass: ...


OURS: EditModule = importlib.import_module("betterlite.videos.main")
OURS_ASYNC: AsyncEditModule = importlib.import_module("betterlite.videos.main")
REFERENCE: EditModule = importlib.import_module("litellm")
REFERENCE_ASYNC: AsyncEditModule = importlib.import_module("litellm")
REFERENCE_ERRORS: ErrorClasses = importlib.import_module("litellm")
OPTIONAL_UTILS: OptionalModule = importlib.import_module("litellm.videos.utils")
REQUESTED_UTILS: RequestedModule = importlib.import_module("litellm.videos.utils")

SYNC_CALLS: Final[dict[str, EditCall]] = {
    "create_character": OURS.video_create_character,
    "get_character": OURS.video_get_character,
    "edit": OURS.video_edit,
    "extension": OURS.video_extension,
}
SYNC_REFERENCES: Final[dict[str, EditCall]] = {
    "create_character": REFERENCE.video_create_character,
    "get_character": REFERENCE.video_get_character,
    "edit": REFERENCE.video_edit,
    "extension": REFERENCE.video_extension,
}
ASYNC_CALLS: Final[dict[str, AsyncEditCall]] = {
    "create_character": OURS_ASYNC.avideo_create_character,
    "get_character": OURS_ASYNC.avideo_get_character,
    "edit": OURS_ASYNC.avideo_edit,
    "extension": OURS_ASYNC.avideo_extension,
}
ASYNC_REFERENCES: Final[dict[str, AsyncEditCall]] = {
    "create_character": REFERENCE_ASYNC.avideo_create_character,
    "get_character": REFERENCE_ASYNC.avideo_get_character,
    "edit": REFERENCE_ASYNC.avideo_edit,
    "extension": REFERENCE_ASYNC.avideo_extension,
}


class Shape(NamedTuple):
    operation: str
    arguments: dict[str, CallValue]
    method: str
    path: str


SHAPES: Final[tuple[Shape, ...]] = (
    Shape(
        "create_character",
        {"name": CHARACTER_NAME, "video": MEDIA_BYTES},
        "POST",
        CHARACTERS_PATH,
    ),
    Shape("get_character", {"character_id": CHARACTER_ID}, "GET", CHARACTER_PATH),
    Shape("edit", {"video_id": VIDEO_ID, "prompt": PROMPT}, "POST", EDITS_PATH),
    Shape(
        "extension",
        {"video_id": VIDEO_ID, "prompt": PROMPT, "seconds": SECONDS},
        "POST",
        EXTENSIONS_PATH,
    ),
)


class Comparison(NamedTuple):
    ours: DumpedObject
    reference: DumpedObject
    ours_call: RecordedCall
    reference_call: RecordedCall


class Upload(NamedTuple):
    method: str
    path: str
    field: str
    text: str
    filename: str


class FixedConfig:
    def map_openai_params(
        self,
        *,
        video_create_optional_params: CreateParams,
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]:
        mapped: dict[str, JsonValue] = {
            "model": model,
            "drop_params": drop_params,
            "extra_body": "stale",
        }
        if "seconds" in video_create_optional_params:
            mapped["seconds"] = video_create_optional_params["seconds"]
        return mapped


def native_cause(error: BaseException) -> NativeVideoEditError:
    cause = error.__cause__
    match cause:
        case NativeVideoEditError():
            return cause
        case object():
            raise AssertionError("the exception does not carry the native error as its cause")


ResultT = TypeVar("ResultT")


async def awaited(awaitable: Awaitable[ResultT]) -> ResultT:
    match await awaitable:
        case result:
            return result


def endpoint_values(server: "EditServer", **arguments: CallValue) -> dict[str, CallValue]:
    values: dict[str, CallValue] = {"custom_llm_provider": PROVIDER, "api_base": server.url}
    values["api_key"] = GOOD_KEY
    values.update(arguments)
    return values


def compared(
    server: "EditServer",
    ours: EditCall,
    reference: EditCall,
    arguments: dict[str, CallValue],
) -> Comparison:
    values = endpoint_values(server, **arguments)
    reference_answer = reference(**values)
    reference_call = server.last()
    server.calls.clear()
    ours_answer = ours(**values)
    return Comparison(ours_answer, reference_answer, server.last(), reference_call)


async def answered_together(
    server: "EditServer",
    ours: AsyncEditCall,
    reference: AsyncEditCall,
    arguments: dict[str, CallValue],
) -> Comparison:
    values = endpoint_values(server, **arguments)
    reference_answer = await reference(**values)
    ours_answer = await ours(**values)
    followed = server.calls[-2:]
    return Comparison(ours_answer, reference_answer, followed[1], followed[0])


def assert_same_call(
    reference_call: RecordedCall, ours_call: RecordedCall, method: str, path: str,
) -> None:
    assert ours_call["method"] == reference_call["method"] == method
    assert ours_call["path"] == reference_call["path"] == path


def assert_same_record(swapped: Comparison) -> None:
    ours = JSON_RECORD.validate_json(swapped.ours.model_dump_json())
    reference = JSON_RECORD.validate_json(swapped.reference.model_dump_json())
    assert ours == reference


class EditServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]

    def __init__(self) -> None:
        super().__init__((HOST, 0), _Handler)
        self.calls = []
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}{API_PREFIX}"

    def record(self, method: str, path: str, headers: dict[str, str], body: bytes) -> None:
        self.calls.append({"method": method, "path": path, "headers": headers, "body": body})

    def last(self) -> RecordedCall:
        return self.calls[-1]

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class _Handler(BaseHTTPRequestHandler):
    owner: ClassVar[EditServer]

    def do_GET(self) -> None:
        self.handle_call()

    def do_POST(self) -> None:
        self.handle_call()

    def handle_call(self) -> None:
        declared = self.headers.get("Content-Length")
        body = b"" if declared is None else self.rfile.read(int(declared))
        self.owner.record(self.command, self.path, dict(self.headers.items()), body)
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply(UNAUTHORIZED, AUTH_BODY)
            return
        self.route(urlsplit(self.path).path)

    def route(self, target: str) -> None:
        command = self.command
        if command == "POST" and target == CHARACTERS_PATH:
            self.reply(OK, CHARACTER_RECORD)
        elif command == "POST" and target == EDITS_PATH:
            self.reply(OK, EDITED_RECORD)
        elif command == "POST" and target == EXTENSIONS_PATH:
            self.reply(OK, EXTENDED_RECORD)
        elif command == "GET" and target == CHARACTER_PATH:
            self.reply(OK, CHARACTER_RECORD)
        else:
            self.reply(NOT_FOUND, MISSING_BODY)

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", JSON_TYPE)
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(OURS.video_create_character)) == parameter_rows(
        inspect.signature(REFERENCE.video_create_character))
    assert parameter_rows(inspect.signature(OURS.avideo_create_character)) == parameter_rows(
        inspect.signature(REFERENCE_ASYNC.avideo_create_character))
    assert parameter_rows(inspect.signature(OURS.video_get_character)) == parameter_rows(
        inspect.signature(REFERENCE.video_get_character))
    assert parameter_rows(inspect.signature(OURS.avideo_get_character)) == parameter_rows(
        inspect.signature(REFERENCE_ASYNC.avideo_get_character))
    assert parameter_rows(inspect.signature(OURS.video_edit)) == parameter_rows(
        inspect.signature(REFERENCE.video_edit))
    assert parameter_rows(inspect.signature(OURS.avideo_edit)) == parameter_rows(
        inspect.signature(REFERENCE_ASYNC.avideo_edit))
    assert parameter_rows(inspect.signature(OURS.video_extension)) == parameter_rows(
        inspect.signature(REFERENCE.video_extension))
    assert parameter_rows(inspect.signature(OURS.avideo_extension)) == parameter_rows(
        inspect.signature(REFERENCE_ASYNC.avideo_extension))
    assert not inspect.iscoroutinefunction(OURS.video_create_character)
    assert not inspect.iscoroutinefunction(OURS.video_get_character)
    assert not inspect.iscoroutinefunction(OURS.video_edit)
    assert not inspect.iscoroutinefunction(OURS.video_extension)
    assert inspect.iscoroutinefunction(OURS_ASYNC.avideo_create_character)
    assert inspect.iscoroutinefunction(OURS_ASYNC.avideo_get_character)
    assert inspect.iscoroutinefunction(OURS_ASYNC.avideo_edit)
    assert inspect.iscoroutinefunction(OURS_ASYNC.avideo_extension)


@pytest.mark.parametrize("shape", SHAPES)
def test_calls_match_litellm(shape: Shape) -> None:
    with closing(EditServer()) as server:
        swapped = compared(
            server, SYNC_CALLS[shape.operation], SYNC_REFERENCES[shape.operation],
            shape.arguments)
        asynchronous = asyncio.run(
            answered_together(
                server, ASYNC_CALLS[shape.operation], ASYNC_REFERENCES[shape.operation],
                shape.arguments))
    assert_same_call(swapped.reference_call, swapped.ours_call, shape.method, shape.path)
    assert_same_call(
        asynchronous.reference_call, asynchronous.ours_call, shape.method, shape.path)
    assert_same_record(swapped)
    assert_same_record(asynchronous)


def check_upload(
    swapped: Comparison, upload: Upload, reference_filename: str | None = None,
) -> None:
    assert_same_call(swapped.reference_call, swapped.ours_call, upload.method, upload.path)
    ours_parts = sent_parts(swapped.ours_call)
    reference_parts = sent_parts(swapped.reference_call)
    expected: Part = {
        "filename": upload.filename, "content_type": MEDIA_TYPE, "body": MEDIA_BYTES,
    }
    reference_expected: Part = {
        "filename": upload.filename if reference_filename is None else reference_filename,
        "content_type": MEDIA_TYPE,
        "body": MEDIA_BYTES,
    }
    assert ours_parts[upload.field]["body"] == reference_parts[upload.field]["body"]
    assert ours_parts[upload.field]["body"] == upload.text.encode()
    assert ours_parts[VIDEO_FIELD] == expected
    assert reference_parts[VIDEO_FIELD] == reference_expected
    assert_same_record(swapped)


def test_character_create_calls() -> None:
    with closing(EditServer()) as server:
        swapped = compared(
            server, SYNC_CALLS["create_character"], SYNC_REFERENCES["create_character"],
            {"name": CHARACTER_NAME, "video": MEDIA_BYTES})
    check_upload(swapped, Upload("POST", CHARACTERS_PATH, NAME_FIELD, CHARACTER_NAME,
                                 MEDIA_FILENAME))


TUPLE_VIDEOS: Final[tuple[tuple[str, bytes] | tuple[str, bytes, str], ...]] = (
    (CLIP_NAME, MEDIA_BYTES),
    (CLIP_NAME, MEDIA_BYTES, MEDIA_TYPE),
)


@pytest.mark.parametrize("video", TUPLE_VIDEOS)
def test_character_tuple_calls(video: tuple[str, bytes] | tuple[str, bytes, str]) -> None:
    with closing(EditServer()) as server:
        OURS.video_create_character(
            **endpoint_values(server, name=CHARACTER_NAME, video=video))
        call = server.last()
    uploaded = sent_parts(call)[VIDEO_FIELD]
    assert uploaded["filename"] == CLIP_NAME
    assert uploaded["content_type"] == MEDIA_TYPE
    assert uploaded["body"] == MEDIA_BYTES


def test_edit_file_calls(tmp_path: Path) -> None:
    source = tmp_path / CLIP_NAME
    source.write_bytes(MEDIA_BYTES)
    with source.open("rb") as clip, closing(EditServer()) as server:
        arguments: dict[str, CallValue] = {
            "video_id": VIDEO_ID, "prompt": PROMPT, "video": clip,
        }
        swapped = compared(server, SYNC_CALLS["edit"], SYNC_REFERENCES["edit"], arguments)
    # LiteLLM sends the open file's own name, the full path, where betterlite sends its basename
    check_upload(swapped, Upload("POST", EDITS_PATH, PROMPT_FIELD, PROMPT, CLIP_NAME),
                 str(source))


def test_extra_body_calls() -> None:
    extra: dict[str, JsonValue] = {"style": STYLE}
    edited: dict[str, CallValue] = {
        "video_id": VIDEO_ID, "prompt": PROMPT, "extra_body": extra,
    }
    extended: dict[str, CallValue] = {**edited, "seconds": SECONDS}
    with closing(EditServer()) as server:
        edit_comparison = compared(
            server, SYNC_CALLS["edit"], SYNC_REFERENCES["edit"], edited)
        extension_comparison = compared(
            server, SYNC_CALLS["extension"], SYNC_REFERENCES["extension"], extended)
        ours_edit = JSON_RECORD.validate_json(edit_comparison.ours_call["body"])
        reference_edit = JSON_RECORD.validate_json(edit_comparison.reference_call["body"])
        ours_extension = JSON_RECORD.validate_json(extension_comparison.ours_call["body"])
        reference_extension = JSON_RECORD.validate_json(
            extension_comparison.reference_call["body"])
    assert ours_edit == reference_edit == {
        PROMPT_FIELD: PROMPT, VIDEO_FIELD: {"id": VIDEO_ID}, "style": STYLE,
    }
    assert ours_extension == reference_extension == {
        PROMPT_FIELD: PROMPT, "seconds": SECONDS, VIDEO_FIELD: {"id": VIDEO_ID},
        "style": STYLE,
    }


def test_package_export_calls() -> None:
    reference: NamesModule = importlib.import_module("litellm.videos")
    ours: NamesModule = importlib.import_module("betterlite.videos")
    main_module: NamesModule = importlib.import_module("betterlite.videos.main")
    names = reference.__all__
    assert set(names) <= set(ours.__all__)
    assert set(names) <= set(main_module.__all__)
    assert set(TYPED_NAMES) <= set(ours.__all__)


def test_utils_optional_calls() -> None:
    optional: CreateParams = CreateParams(
        seconds=SECONDS, model=MODEL, extra_body={"style": STYLE})
    ours = map_video_optional_params(MODEL, FixedConfig(), optional)
    reference = OPTIONAL_UTILS.VideoGenerationRequestUtils.get_optional_params_video_generation(
        MODEL, FixedConfig(), optional)
    expected: dict[str, JsonValue] = {
        "model": MODEL, "seconds": SECONDS, "drop_params": False, "style": STYLE,
    }
    assert ours == reference == expected


def test_utils_requested_calls() -> None:
    ours_params: dict[str, JsonValue] = copy.deepcopy(REQUESTED_PARAMS)
    reference_params: dict[str, JsonValue] = copy.deepcopy(REQUESTED_PARAMS)
    ours = requested_optional_params(ours_params)
    reference_utils = REQUESTED_UTILS.VideoGenerationRequestUtils
    reference = reference_utils.get_requested_video_generation_optional_param(reference_params)
    assert ours == reference
    assert "timeout" not in ours
    assert ours["style"] == "loose"
    assert ours["second"] == "2"
    assert "extra_body" in ours


def test_error_matches_litellm() -> None:
    reference_class = REFERENCE_ERRORS.AuthenticationError
    with closing(EditServer()) as server:
        values = endpoint_values(server, video_id=VIDEO_ID, prompt=PROMPT, api_key=BAD_KEY)
        with pytest.raises(reference_class):
            REFERENCE.video_edit(**values)
        with pytest.raises(AuthenticationError) as made:
            OURS.video_edit(**values)
        with pytest.raises(AuthenticationError) as asynchronous:
            asyncio.run(awaited(OURS_ASYNC.avideo_edit(**values)))
        read_values = endpoint_values(server, character_id=CHARACTER_ID, api_key=BAD_KEY)
        with pytest.raises(reference_class):
            REFERENCE.video_get_character(**read_values)
        with pytest.raises(AuthenticationError) as looked_up:
            OURS.video_get_character(**read_values)
    assert made.value.status_code == asynchronous.value.status_code == UNAUTHORIZED
    assert looked_up.value.status_code == UNAUTHORIZED
    payload = ERROR_PAYLOAD.validate_json(str(native_cause(made.value)))
    assert payload["type"] == reference_class.__name__
    assert "status_code" in payload
    assert payload["status_code"] == UNAUTHORIZED


def test_refused_provider_calls() -> None:
    with closing(EditServer()) as server:
        character_values = endpoint_values(
            server, name=CHARACTER_NAME, video=MEDIA_BYTES,
            custom_llm_provider=REFUSED_CHARACTER_PROVIDER)
        with pytest.raises(BadRequestError) as created:
            OURS.video_create_character(**character_values)
        edit_values = endpoint_values(
            server, video_id=VIDEO_ID, prompt=PROMPT,
            custom_llm_provider=REFUSED_EDIT_PROVIDER)
        with pytest.raises(BadRequestError) as edited:
            OURS.video_edit(**edit_values)
        with pytest.raises(BadRequestError) as extended:
            OURS.video_extension(**{**edit_values, "seconds": SECONDS})
    assert created.value.status_code == edited.value.status_code == BAD_REQUEST
    assert extended.value.status_code == BAD_REQUEST
    assert server.calls == []


def test_character_mock_calls() -> None:
    create_arguments: dict[str, CallValue] = {
        "name": CHARACTER_NAME,
        "video": MEDIA_BYTES,
        "custom_llm_provider": REFUSED_CHARACTER_PROVIDER,
        "mock_response": MOCKED_CHARACTER,
    }
    get_arguments: dict[str, CallValue] = {
        "character_id": CHARACTER_ID,
        "custom_llm_provider": REFUSED_CHARACTER_PROVIDER,
        "mock_response": MOCKED_CHARACTER,
    }
    with closing(EditServer()) as server:
        create_values = endpoint_values(server, **create_arguments)
        get_values = endpoint_values(server, **get_arguments)
        reference_answers = [
            REFERENCE.video_create_character(**create_values),
            REFERENCE.video_get_character(**get_values),
            asyncio.run(awaited(REFERENCE_ASYNC.avideo_create_character(**create_values))),
            asyncio.run(awaited(REFERENCE_ASYNC.avideo_get_character(**get_values))),
        ]
        ours_answers = [
            OURS.video_create_character(**create_values),
            OURS.video_get_character(**get_values),
            asyncio.run(awaited(OURS_ASYNC.avideo_create_character(**create_values))),
            asyncio.run(awaited(OURS_ASYNC.avideo_get_character(**get_values))),
        ]
    reference_records = [
        JSON_RECORD.validate_json(answer.model_dump_json()) for answer in reference_answers
    ]
    ours_records = [
        JSON_RECORD.validate_json(answer.model_dump_json()) for answer in ours_answers
    ]
    assert server.calls == []
    assert ours_records == reference_records == [MOCKED_CHARACTER] * len(ours_records)


def test_unsupported_objects() -> None:
    with pytest.raises(UnsupportedCallArgument, match="client"):
        OURS.video_create_character(
            name=CHARACTER_NAME, video=MEDIA_BYTES, client="client")
    with pytest.raises(UnsupportedCallArgument, match="client"):
        OURS.video_get_character(character_id=CHARACTER_ID, client="client")
    with pytest.raises(UnsupportedCallArgument, match="client"):
        OURS.video_edit(video_id=VIDEO_ID, prompt=PROMPT, client="client")
    with pytest.raises(UnsupportedCallArgument, match="client"):
        OURS.video_extension(
            video_id=VIDEO_ID, prompt=PROMPT, seconds=SECONDS, client="client")
    with pytest.raises(UnsupportedCallArgument, match="client"):
        asyncio.run(awaited(OURS_ASYNC.avideo_create_character(
            name=CHARACTER_NAME, video=MEDIA_BYTES, client="client")))
    with pytest.raises(UnsupportedCallArgument, match="client"):
        asyncio.run(awaited(
            OURS_ASYNC.avideo_get_character(character_id=CHARACTER_ID, client="client")))
    with pytest.raises(UnsupportedCallArgument, match="client"):
        asyncio.run(awaited(
            OURS_ASYNC.avideo_edit(video_id=VIDEO_ID, prompt=PROMPT, client="client")))
    with pytest.raises(UnsupportedCallArgument, match="client"):
        asyncio.run(awaited(OURS_ASYNC.avideo_extension(
            video_id=VIDEO_ID, prompt=PROMPT, seconds=SECONDS, client="client")))
    with pytest.raises(UnsupportedCallArgument, match="shared_session"):
        OURS.video_edit(video_id=VIDEO_ID, prompt=PROMPT, shared_session="session")
    with pytest.raises(UnsupportedCallArgument, match="shared_session"):
        asyncio.run(awaited(OURS_ASYNC.avideo_extension(
            video_id=VIDEO_ID, prompt=PROMPT, seconds=SECONDS, shared_session="session")))
