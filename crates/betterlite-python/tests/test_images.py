import asyncio
import importlib
import inspect
import io
import json
import logging
import threading
from contextlib import closing
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Protocol

import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._images import main as images_main
from betterlite.exceptions import (
    APIConnectionError,
    AuthenticationError,
    BadRequestError,
    UnsupportedParamsError,
)
from betterlite.main import (
    aimage_edit,
    aimage_generation,
    aimage_variation,
    image_edit,
    image_generation,
    image_variation,
)
from betterlite.types.utils.media import ImageResponse

HOST: Final = "127.0.0.1"
OK: Final = 200
UNAUTHORIZED: Final = 401
GOOD_KEY: Final = "sk-test"
BAD_KEY: Final = "bad-key"
DEFAULT_MODEL: Final = "dall-e-2"
PROVIDER: Final = "openai"
GENERATIONS_PATH: Final = "/v1/images/generations"
VARIATIONS_PATH: Final = "/v1/images/variations"
EDITS_PATH: Final = "/v1/images/edits"
API_BASE_PATH: Final = "/v1"
JSON_TYPE: Final = "application/json"
B64: Final = "aW1hZ2UtYnl0ZXM="
REVISED: Final = "a revised prompt"
PROMPT: Final = "draw a cat"
AUTH_TEXT: Final = "Incorrect API key provided: bad-key"
PNG_BYTES: Final = b"\x89PNG\r\n\x1a\nfirst"
JPEG_BYTES: Final = b"\xff\xd8\xff\xe0second"
MASK_BYTES: Final = b"\x89PNG\r\n\x1a\nmask"
SEED: Final = 7
CREATED: Final = 1700000000
AUTH_BODY: Final[dict[str, JsonValue]] = {
    "error": {
        "message": AUTH_TEXT,
        "type": "invalid_request_error",
        "param": None,
        "code": "invalid_api_key",
    }
}
IMAGE_BODY: Final[dict[str, JsonValue]] = {
    "created": CREATED,
    "data": [{"b64_json": B64, "revised_prompt": REVISED}],
    "usage": {
        "input_tokens": 1,
        "input_tokens_details": {"image_tokens": 1, "text_tokens": 0},
        "output_tokens": 2,
        "total_tokens": 3,
    },
}

BODY: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class RecordedCall(TypedDict):
    path: str
    headers: dict[str, str]
    body: bytes


class ReferenceCall(Protocol):
    def __call__(self, *args: JsonValue, **kwargs: JsonValue) -> JsonValue: ...


class ReferenceModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


REFERENCE: ReferenceModule = importlib.import_module("litellm")


class NamedFile(io.BytesIO):
    def __init__(self, data: bytes, name: str) -> None:
        super().__init__(data)
        self.name = name


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows


class ImageServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]

    def __init__(self) -> None:
        super().__init__((HOST, 0), ImageHandler)
        self.calls = []
        ImageHandler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}{API_BASE_PATH}"

    def record(self, path: str, headers: dict[str, str], body: bytes) -> None:
        self.calls.append({"path": path, "headers": headers, "body": body})

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class ImageHandler(BaseHTTPRequestHandler):
    owner: ClassVar[ImageServer]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        raw = b"" if declared is None else self.rfile.read(int(declared))
        self.owner.record(self.path, dict(self.headers.items()), raw)
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply(UNAUTHORIZED, AUTH_BODY)
            return
        self.reply(OK, dict(IMAGE_BODY))

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", JSON_TYPE)
        self.send_header("x-request-id", "image-request")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def header_of(call: RecordedCall, name: str) -> str | None:
    for header, value in call["headers"].items():
        if header.lower() == name.lower():
            return value
    return None


def offset(body: bytes, needle: bytes) -> int:
    found = body.find(needle)
    assert found >= 0
    return found


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(image_generation)) == parameter_rows(
        inspect.signature(REFERENCE.image_generation)
    )
    assert parameter_rows(inspect.signature(aimage_generation)) == parameter_rows(
        inspect.signature(REFERENCE.aimage_generation)
    )
    assert parameter_rows(inspect.signature(image_variation)) == parameter_rows(
        inspect.signature(REFERENCE.image_variation)
    )
    assert parameter_rows(inspect.signature(aimage_variation)) == parameter_rows(
        inspect.signature(REFERENCE.aimage_variation)
    )
    assert parameter_rows(inspect.signature(image_edit)) == parameter_rows(
        inspect.signature(REFERENCE.image_edit)
    )
    assert parameter_rows(inspect.signature(aimage_edit)) == parameter_rows(
        inspect.signature(REFERENCE.aimage_edit)
    )
    assert not inspect.iscoroutinefunction(image_generation)
    assert inspect.iscoroutinefunction(aimage_generation)


def test_lazy_edit_helper_absent() -> None:
    with pytest.raises(AttributeError):
        _ = images_main.ImageEditRequestUtils


def test_generate_returns_response() -> None:
    with closing(ImageServer()) as server:
        response = image_generation(
            prompt=PROMPT,
            model=DEFAULT_MODEL,
            quality="hd",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    body = BODY.validate_json(call["body"])
    assert call["path"] == GENERATIONS_PATH
    assert body["model"] == DEFAULT_MODEL
    assert body["prompt"] == PROMPT
    assert body["quality"] == "hd"
    assert header_of(call, "authorization") == f"Bearer {GOOD_KEY}"
    assert response.created == CREATED
    first = response.data[0]
    assert first.b64_json == B64
    assert first.revised_prompt == REVISED
    hidden = response._hidden_params  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L233

    assert hidden["model"] == DEFAULT_MODEL
    assert BODY.validate_python(hidden["headers"])["x-request-id"] == "image-request"
    additional = BODY.validate_python(hidden["additional_headers"])
    assert additional["llm_provider-x-request-id"] == "image-request"


def test_generate_ignores_mock() -> None:
    with closing(ImageServer()) as server:
        response = image_generation(
            prompt=PROMPT,
            api_base=server.url,
            api_key=GOOD_KEY,
            mock_response="mocked",
        )
        calls = server.calls
    assert len(calls) == 1
    assert response.data[0].b64_json == B64


async def generate_call() -> tuple[ImageResponse, RecordedCall]:
    with closing(ImageServer()) as server:
        response = await aimage_generation(
            prompt=PROMPT, model=DEFAULT_MODEL, api_base=server.url, api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    return response, call


def test_agenerate_returns_reply() -> None:
    response, call = asyncio.run(generate_call())
    assert BODY.validate_json(call["body"])["model"] == DEFAULT_MODEL
    assert response.data[0].b64_json == B64
    assert response._hidden_params["model"] == DEFAULT_MODEL  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L233


def test_edit_sends_files_in_order() -> None:
    with closing(ImageServer()) as server:
        response = image_edit(
            image=[NamedFile(PNG_BYTES, "first.png"), NamedFile(JPEG_BYTES, "second.jpg")],
            prompt=PROMPT,
            model="gpt-image-1",
            mask=NamedFile(MASK_BYTES, "mask.png"),
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    body = call["body"]
    assert call["path"] == EDITS_PATH
    assert offset(body, b'name="prompt"') < offset(body, b'name="image[]"')
    first = offset(body, b'filename="first.png"')
    second = offset(body, b'filename="second.jpg"')
    mask = offset(body, b'filename="mask.png"')
    assert first < second < mask
    assert offset(body, b"Content-Type: image/png") < second
    assert offset(body, b"Content-Type: image/jpeg") > first
    assert offset(body, b'name="mask"') < mask
    assert response.data[0].b64_json == B64
    assert response._hidden_params == {}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L792


def test_edit_rejects_two_images() -> None:
    images = [NamedFile(PNG_BYTES, "first.png"), NamedFile(JPEG_BYTES, "second.jpg")]
    with closing(ImageServer()) as server, pytest.raises(
        BadRequestError, match="DALL-E-2 only supports editing a single image"
    ):
        image_edit(
            image=images, prompt=PROMPT, model=DEFAULT_MODEL,
            api_base=server.url, api_key=GOOD_KEY,
        )
    assert server.calls == []


def test_variation_sends_image() -> None:
    with closing(ImageServer()) as server:
        response = image_variation(
            image=NamedFile(JPEG_BYTES, "photo.jpg"),
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    body = call["body"]
    assert call["path"] == VARIATIONS_PATH
    assert b'name="image"' in body
    assert b'filename="photo.jpg"' in body
    assert b"Content-Type: image/jpeg" in body
    assert response.data[0].revised_prompt == REVISED
    assert response._hidden_params == {}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L648


def test_aedit_sends_the_image() -> None:
    with closing(ImageServer()) as server:
        response = asyncio.run(
            aimage_edit(
                image=NamedFile(PNG_BYTES, "first.png"),
                model=DEFAULT_MODEL,
                prompt=PROMPT,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        call = server.calls[-1]
    assert call["path"] == EDITS_PATH
    assert response.data[0].b64_json == B64


def test_edit_rejects_image_config() -> None:
    with closing(ImageServer()) as server, pytest.raises(UnsupportedParamsError) as caught:
        image_edit(
            image=NamedFile(PNG_BYTES, "first.png"),
            model=DEFAULT_MODEL,
            prompt=PROMPT,
            imageConfig={"quality": "hd"},
            api_base=server.url,
            api_key=GOOD_KEY,
        )
    assert "imageConfig" in caught.value.message
    assert server.calls == []


def test_edit_drop_params_drops() -> None:
    with closing(ImageServer()) as server:
        image_edit(
            image=NamedFile(PNG_BYTES, "first.png"),
            model=DEFAULT_MODEL,
            prompt=PROMPT,
            imageConfig={"quality": "hd"},
            drop_params=True,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    assert b"imageConfig" not in call["body"]


def test_generate_error_maps_class() -> None:
    with closing(ImageServer()) as server, pytest.raises(AuthenticationError) as caught:
        image_generation(
            prompt=PROMPT,
            model=DEFAULT_MODEL,
            api_base=server.url,
            api_key=BAD_KEY,
        )
    error = caught.value
    assert error.status_code == UNAUTHORIZED
    assert error.model == DEFAULT_MODEL
    assert error.llm_provider == PROVIDER
    assert AUTH_TEXT in error.message
    cause = error.__cause__
    assert cause is not None
    assert cause.__class__.__name__ == "NativeImageError"


def test_variation_bad_provider() -> None:
    with pytest.raises(ValueError, match="Invalid image variation provider"):
        image_variation(image=NamedFile(JPEG_BYTES, "photo.jpg"), model="vertex_ai/foo")


def test_avariation_awaitable() -> None:
    with closing(ImageServer()) as server:
        pending = aimage_variation(
            NamedFile(JPEG_BYTES, "photo.jpg"), api_base=server.url, api_key=GOOD_KEY,
        )
        assert inspect.isawaitable(pending)
        response = asyncio.run(pending)
    assert response.data[0].b64_json == B64


def test_agenerate_requires_model() -> None:
    with pytest.raises(KeyError, match="model"):
        asyncio.run(aimage_generation(prompt=PROMPT))


@pytest.mark.parametrize("drop", [False, True])
def test_generate_params_drop(drop: bool) -> None:
    with closing(ImageServer()) as server:
        if drop:
            image_generation(
                prompt=PROMPT, model=DEFAULT_MODEL, style="vivid", drop_params=True,
                api_base=server.url, api_key=GOOD_KEY,
            )
            assert "style" not in BODY.validate_json(server.calls[-1]["body"])
        else:
            with pytest.raises(UnsupportedParamsError):
                image_generation(
                    prompt=PROMPT, model=DEFAULT_MODEL, style="vivid",
                    api_base=server.url, api_key=GOOD_KEY,
                )
            assert server.calls == []


@pytest.mark.parametrize("extra", [{"extra_body": {"seed": 7}}, {"seed": 7}])
def test_generate_params_extra(extra: dict[str, JsonValue]) -> None:
    with closing(ImageServer()) as server:
        image_generation(
            prompt=PROMPT, model="dall-e-3", api_base=server.url, api_key=GOOD_KEY,
            **extra,
        )
        body = BODY.validate_json(server.calls[-1]["body"])
    assert body["seed"] == SEED
    assert "extra_body" not in body


def test_edit_caller_authorization() -> None:
    with closing(ImageServer()) as server:
        response = image_edit(
            image=NamedFile(PNG_BYTES, "cat.png"), prompt=PROMPT, model=DEFAULT_MODEL,
            api_base=server.url, api_key=BAD_KEY,
            extra_headers={"Authorization": f"Bearer {GOOD_KEY}"},
        )
        call = server.calls[-1]
    assert header_of(call, "authorization") == f"Bearer {GOOD_KEY}"
    assert response.created == CREATED


@pytest.mark.parametrize("name", ["client", "shared_session"])
def test_generate_refuses_client(name: str) -> None:
    with pytest.raises(UnsupportedCallArgument) as caught:
        image_generation(prompt=PROMPT, **{name: object()})
    assert caught.value.argument == name


def test_generate_drops_logger(caplog: pytest.LogCaptureFixture) -> None:
    with closing(ImageServer()) as server, caplog.at_level(logging.WARNING, logger="LiteLLM"):
        response = image_generation(
            prompt=PROMPT, logger_fn=print, api_base=server.url, api_key=GOOD_KEY,
        )
        body = BODY.validate_json(server.calls[-1]["body"])
    assert response.created == CREATED
    assert "logger_fn" not in body
    assert "logger_fn is not called" in caplog.text


def test_avariation_missing_key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    monkeypatch.setattr("betterlite.api_key", None)
    monkeypatch.setattr("betterlite.openai_key", None)
    with pytest.raises(APIConnectionError) as caught:
        asyncio.run(aimage_variation(image=NamedFile(PNG_BYTES, "cat.png")))
    assert caught.value.status_code == HTTPStatus.INTERNAL_SERVER_ERROR
