import itertools
import json
import os
from collections.abc import Callable
from types import SimpleNamespace
from typing import Literal, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.llms.runwayml.image_generation import transformation as image_module
from litellm.llms.runwayml.image_generation.transformation import RunwayMLImageGenerationConfig
from litellm.llms.runwayml.text_to_speech import transformation as speech_module
from litellm.llms.runwayml.text_to_speech.transformation import RunwayMLTextToSpeechConfig
from pydantic import BaseModel, ConfigDict, JsonValue

Json: TypeAlias = JsonValue
Kind: TypeAlias = Literal["image", "speech"]


class Reply(BaseModel):
    model_config = ConfigDict(extra="forbid")
    status: int
    body: str


class Headers(BaseModel):
    model_config = ConfigDict(extra="forbid")
    kind: Kind
    environment: dict[str, str]
    headers: dict[str, str]
    api_key: str | None


class Endpoint(BaseModel):
    model_config = ConfigDict(extra="forbid")
    kind: Kind
    environment: dict[str, str]
    api_base: str | None


class Poll(BaseModel):
    model_config = ConfigDict(extra="forbid")
    kind: Kind
    task_id: str
    api_base: str
    timeout_seconds: int
    step_millis: int
    replies: list[Reply]


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")
    operation: Literal["headers", "endpoint", "poll"]
    params: dict[str, Json]


class ScriptedClient:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.urls: list[str] = []

    def get(self, url: str, headers: dict[str, str]) -> httpx.Response:
        self.urls.append(url)
        reply = self.replies.pop(0)
        request = httpx.Request("GET", url, headers=headers)
        return httpx.Response(reply.status, content=reply.body.encode(), request=request)


def headers(params: dict[str, Json]) -> Json:
    request = Headers.model_validate(params)
    image = RunwayMLImageGenerationConfig()
    speech = RunwayMLTextToSpeechConfig()
    validate_image = cast(Callable[..., dict[str, str]], image.validate_environment)  # pyright: ignore[reportUnknownMemberType]
    validate_speech = cast(Callable[..., dict[str, str]], speech.validate_environment)  # pyright: ignore[reportUnknownMemberType]
    with patch.dict(os.environ, request.environment, clear=True):
        try:
            if request.kind == "image":
                sent = validate_image(request.headers, "model", [], {}, {}, api_key=request.api_key)
            else:
                sent = validate_speech(request.headers, "model", request.api_key)
        except ValueError as error:
            return {"error": "other", "message": str(error)}
    return {"headers": {name.lower(): value for name, value in sent.items()}}


def endpoint(params: dict[str, Json]) -> Json:
    request = Endpoint.model_validate(params)
    image_url = cast(Callable[..., str], RunwayMLImageGenerationConfig().get_complete_url)  # pyright: ignore[reportUnknownMemberType]
    speech_url = cast(Callable[..., str], RunwayMLTextToSpeechConfig().get_complete_url)  # pyright: ignore[reportUnknownMemberType]
    with patch.dict(os.environ, request.environment, clear=True):
        if request.kind == "image":
            return {"url": image_url(request.api_base, None, "model", {}, {})}
        return {"url": speech_url("model", request.api_base, {})}


def poll(params: dict[str, Json]) -> Json:
    request = Poll.model_validate(params)
    client = ScriptedClient(request.replies)
    sleeps: list[int] = []
    readings = itertools.count(0, request.step_millis)
    clock = SimpleNamespace(
        time=lambda: next(readings) / 1000,
        sleep=sleeps.append,
    )
    if request.kind == "image":
        module, config = image_module, RunwayMLImageGenerationConfig()
    else:
        module, config = speech_module, RunwayMLTextToSpeechConfig()
    answer: dict[str, Json]
    with (
        patch.object(module, "time", clock),
        patch("litellm.llms.custom_httpx.http_handler._get_httpx_client", return_value=client),
    ):
        try:
            final = config._poll_task_sync(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                request.task_id, request.api_base, {}, request.timeout_seconds
            )
            task = cast(dict[str, Json], json.loads(final.text))
            answer = {"task": {"id": task.get("id"), "status": task.get("status")}}
        except TimeoutError as error:
            answer = {"error": "timeout", "message": str(error)}
        except httpx.HTTPStatusError as error:
            answer = {"error": "http_status", "status": error.response.status_code}
        except json.JSONDecodeError:
            answer = {"error": "decode"}
        except ValueError as error:
            answer = {"error": "task", "message": str(error)}
    return {"outcome": answer, "gets": list(client.urls), "sleeps": list(sleeps)}


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "headers": headers,
    "endpoint": endpoint,
    "poll": poll,
}


def run(argument: Json) -> Json:
    supplied = Input.model_validate(argument)
    _ = litellm
    return HANDLERS[supplied.operation](supplied.params)
