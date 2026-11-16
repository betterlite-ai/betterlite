import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from pydantic import JsonValue

from .auth import activate
from .views import CHAT
from .wire import ModelCall


class RecordingTransport(httpx.MockTransport):
    def __init__(self, status: int, body: str) -> None:
        super().__init__(self.answer)
        self.status = status
        self.body = body
        self.url: str | None = None
        self.authorization: str | None = None

    def answer(self, request: httpx.Request) -> httpx.Response:
        self.url = str(request.url)
        self.authorization = request.headers.get("authorization")
        return httpx.Response(status_code=self.status, content=self.body.encode())


def model_value(call: ModelCall) -> JsonValue:
    activate(call["environment"])
    recorder = RecordingTransport(call["status"], call["body"])
    litellm.module_level_client = HTTPHandler(client=httpx.Client(transport=recorder))
    models: list[JsonValue] = [*CHAT.get_models(call["api_key"], call["api_base"])]
    return {
        "models": models,
        "url": recorder.url,
        "authorization": recorder.authorization,
    }
