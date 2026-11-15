from typing import TypedDict

import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from pydantic import JsonValue, TypeAdapter

from .boundary import CHAT, VALUE
from .environment import install_environment


class ModelsCase(TypedDict):
    api_base: str | None
    api_key: str | None
    environment_base: str | None
    environment_key: str | None
    openai_api_key: str | None
    reply_status: int
    reply_body: str


class Recorder:
    def __init__(self, *, status: int, body: str) -> None:
        self.status = status
        self.body = body
        self.url: str | None = None
        self.authorization: str | None = None

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.url = str(request.url)
        self.authorization = request.headers.get("authorization")
        return httpx.Response(self.status, text=self.body, request=request)


def models_value(case: ModelsCase) -> JsonValue:
    install_environment(
        base=case["environment_base"],
        key=case["environment_key"],
        forced=None,
        openai_key=case["openai_api_key"],
    )
    recorder = Recorder(status=case["reply_status"], body=case["reply_body"])
    client = httpx.Client(transport=httpx.MockTransport(recorder.handle))
    litellm.module_level_client = HTTPHandler(client=client)
    try:
        models = CHAT.get_models(api_key=case["api_key"], api_base=case["api_base"])
    except ValueError:
        return {"ok": False}
    return {
        "ok": True,
        "models": VALUE.validate_python(models),
        "url": recorder.url,
        "authorization": recorder.authorization,
    }


def models_cases(params: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[ModelsCase]).validate_python(params)
    return {"results": [models_value(case) for case in cases]}
