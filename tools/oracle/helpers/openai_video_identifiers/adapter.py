import logging
from collections.abc import Callable, Mapping
from types import TracebackType
from typing import Annotated, Final, Literal, Protocol, Self, TypeAlias, TypedDict

from litellm.llms.openai.videos.transformation import OpenAIVideoConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.utils import (
    decode_character_id_with_provider,
    decode_video_id_with_provider,
    encode_character_id_with_provider,
    encode_video_id_with_provider,
    extract_original_character_id,
    extract_original_video_id,
)
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from .fixtures import Content, Fixture, FixtureState, Part, Parts, StoredFiles, fixtures

LOGGER_NAME: Final = "LiteLLM"
API_BASE: Final = "https://mock.example/v1"
MODEL: Final = "sora-2"
PROMPT: Final = "make a video"
VIDEO_ID: Final = "vid-1"
DEFAULT_FIELD: Final = "input_reference"
# LiteLLM declares the edit form data as Mapping[str, object]; the helper never reads it
EditForm: TypeAlias = Mapping[str, object]
EditResult: TypeAlias = tuple[str, EditForm, Parts | None]


class VideoConfig(Protocol):
    @property
    def transform_video_create_request(
        self,
    ) -> Callable[
        [
            str,
            str,
            str,
            dict[str, JsonValue] | dict[str, StoredFiles],
            GenericLiteLLMParams,
            dict[str, str],
        ],
        tuple[dict[str, JsonValue], Parts, str],
    ]: ...

    @property
    def transform_video_edit_request(
        self,
    ) -> Callable[
        [str, str, str, GenericLiteLLMParams, dict[str, str], Content | None],
        EditResult,
    ]: ...


CONFIG: VideoConfig = OpenAIVideoConfig()


class Decoded(TypedDict):
    custom_llm_provider: str | None
    model_id: str | None
    video_id: str


class DecodedCharacter(TypedDict):
    custom_llm_provider: str | None
    model_id: str | None
    character_id: str


DECODED: TypeAdapter[Decoded] = TypeAdapter(Decoded)
DECODED_CHARACTER: TypeAdapter[DecodedCharacter] = TypeAdapter(DecodedCharacter)
TEXT: TypeAdapter[str] = TypeAdapter(str)


class EncodeVideoId(TypedDict):
    op: Literal["encode_video_id"]
    video_id: str
    provider: str
    model_id: str | None


class DecodeVideoId(TypedDict):
    op: Literal["decode_video_id"]
    encoded: str
    capture_debug: bool


class ExtractVideoId(TypedDict):
    op: Literal["extract_original_video_id"]
    encoded: str
    capture_debug: bool


class EncodeCharacterId(TypedDict):
    op: Literal["encode_character_id"]
    character_id: str
    provider: str
    model_id: str | None


class DecodeCharacterId(TypedDict):
    op: Literal["decode_character_id"]
    encoded: str
    capture_debug: bool


class ExtractCharacterId(TypedDict):
    op: Literal["extract_original_character_id"]
    encoded: str
    capture_debug: bool


class RewriteCharacters(TypedDict):
    op: Literal["rewrite_characters"]
    request: dict[str, JsonValue]


class AddImageToFiles(TypedDict):
    op: Literal["add_image_to_files"]
    fixture: Fixture


class VideoFileTuple(TypedDict):
    op: Literal["video_file_tuple"]
    fixture: Fixture


Call: TypeAlias = Annotated[
    EncodeVideoId | DecodeVideoId | ExtractVideoId | EncodeCharacterId | DecodeCharacterId
    | ExtractCharacterId | RewriteCharacters | AddImageToFiles | VideoFileTuple,
    Field(discriminator="op"),
]
CALLS: TypeAdapter[list[Call]] = TypeAdapter(list[Call])


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.DEBUG)
        self.logger = logging.getLogger(LOGGER_NAME)
        self.handlers = self.logger.handlers
        self.level = self.logger.level
        self.events: list[str] = []

    def __enter__(self) -> Self:
        self.logger.handlers = [self]
        self.logger.setLevel(logging.DEBUG)
        return self

    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.logger.handlers = self.handlers
        self.logger.setLevel(self.level)

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno == logging.DEBUG:
            self.events.append(record.getMessage())


def decode_json(decoded: Decoded) -> dict[str, JsonValue]:
    return {
        "custom_llm_provider": decoded["custom_llm_provider"],
        "model_id": decoded["model_id"],
        "video_id": decoded["video_id"],
    }


def decode_character_json(decoded: DecodedCharacter) -> dict[str, JsonValue]:
    return {
        "custom_llm_provider": decoded["custom_llm_provider"],
        "model_id": decoded["model_id"],
        "character_id": decoded["character_id"],
    }


def rewrite_call(call: RewriteCharacters) -> JsonValue:
    request = call["request"]
    model = TEXT.validate_python(request["model"])
    prompt = TEXT.validate_python(request["prompt"])
    data, _files, _base = CONFIG.transform_video_create_request(
        model, prompt, API_BASE, request, GenericLiteLLMParams(), {}
    )
    return data


def parts_call(files: Parts | None, state: FixtureState) -> JsonValue:
    match files:
        case [(field, (filename, content, content_type)), *_]:
            part: Part = (field, (filename, content, content_type))
            return state.result([part])
        case _:
            raise ValueError("the transform returned no multipart part")


def add_image_call(call: AddImageToFiles) -> JsonValue:
    state = fixtures(call["fixture"])
    _data, files, _base = CONFIG.transform_video_create_request(
        MODEL, PROMPT, API_BASE, {DEFAULT_FIELD: state.content}, GenericLiteLLMParams(), {}
    )
    return parts_call(files, state)


def video_call(call: VideoFileTuple) -> JsonValue:
    state = fixtures(call["fixture"])
    _url, _form, files = CONFIG.transform_video_edit_request(
        PROMPT, VIDEO_ID, API_BASE, GenericLiteLLMParams(), {}, state.content
    )
    return parts_call(files, state)


def id_call(call: Call) -> JsonValue:
    match call["op"]:
        case "encode_video_id":
            return encode_video_id_with_provider(
                call["video_id"], call["provider"], call["model_id"]
            )
        case "decode_video_id":
            with Capture() as capture:
                decoded = DECODED.validate_python(decode_video_id_with_provider(call["encoded"]))
            result = decode_json(decoded)
            result["logged"] = bool(capture.events)
            return result
        case "extract_original_video_id":
            with Capture() as capture:
                identifier = extract_original_video_id(call["encoded"])
            return {"id": identifier, "logged": bool(capture.events)}
        case "encode_character_id":
            return encode_character_id_with_provider(
                call["character_id"], call["provider"], call["model_id"]
            )
        case "decode_character_id":
            with Capture() as capture:
                decoded = DECODED_CHARACTER.validate_python(
                    decode_character_id_with_provider(call["encoded"])
                )
            result = decode_character_json(decoded)
            result["logged"] = bool(capture.events)
            return result
        case "extract_original_character_id":
            with Capture() as capture:
                identifier = extract_original_character_id(call["encoded"])
            return {"id": identifier, "logged": bool(capture.events)}
        case _:
            raise ValueError(f"not an identifier operation: {call['op']}")


def run_call(call: Call) -> JsonValue:
    match call["op"]:
        case "rewrite_characters":
            return rewrite_call(call)
        case "add_image_to_files":
            return add_image_call(call)
        case "video_file_tuple":
            return video_call(call)
        case "encode_video_id" | "decode_video_id" | "extract_original_video_id":
            return id_call(call)
        case "encode_character_id" | "decode_character_id" | "extract_original_character_id":
            return id_call(call)


def run(raw: dict[str, JsonValue]) -> JsonValue:
    calls = CALLS.validate_python(raw["calls"])
    results: list[JsonValue] = [run_call(call) for call in calls]
    return results
