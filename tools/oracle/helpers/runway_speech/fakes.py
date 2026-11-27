import httpx
from pydantic import JsonValue, TypeAdapter

from .inputs import Object, Reply

PARAMS: TypeAdapter[Object] = TypeAdapter(Object)


class Clock:
    def __init__(self, step_millis: int) -> None:
        self.millis = 0
        self.step_millis = step_millis
        self.sleeps: list[JsonValue] = []

    def time(self) -> float:
        current = self.millis
        self.millis += self.step_millis
        return current / 1000.0

    def sleep(self, seconds: float) -> None:
        pause = int(seconds)
        self.sleeps.append(pause)


class Scripted:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        if not self.replies:
            return httpx.Response(500, content=b"")
        reply = self.replies.pop(0)
        return httpx.Response(
            reply["status"], headers=reply["headers"], content=reply["body"].encode()
        )


class Recorder:
    def __init__(self) -> None:
        self.call: Object = {}

    def text_to_speech_handler(self, **kwargs: JsonValue) -> JsonValue:
        params = PARAMS.validate_python(kwargs["litellm_params"])
        call: Object = {
            "model": kwargs["model"],
            "input": kwargs["input"],
            "voice": kwargs["voice"],
            "optional_params": kwargs["text_to_speech_optional_params"],
            "litellm_params": params,
            "api_base": params["api_base"],
            "api_key": params["api_key"],
            "timeout_seconds": kwargs["timeout"],
            "extra_headers": kwargs["extra_headers"],
            "aspeech": kwargs["_is_async"],
            "client": kwargs["client"],
        }
        self.call = call
        return call
