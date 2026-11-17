from typing import Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.llms.vertex_ai.gemini import transformation
from litellm.llms.vertex_ai.gemini.transformation import (
    _gemini_convert_messages_with_history as convert_history,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L691-L696
)
from openai import BadRequestError
from pydantic import JsonValue as Json

JsonObject: TypeAlias = dict[str, Json]

litellm.suppress_debug_info = True


class Metadata(TypedDict):
    status: int
    json: Json
    body: str | None
    connect: bool


class Case(TypedDict):
    messages: list[JsonObject]
    model: str
    litellm_params: JsonObject
    metadata: Metadata
    mint_error: bool


class Convert(Protocol):
    def __call__(
        self,
        messages: list[JsonObject],
        *,
        model: str,
        litellm_params: JsonObject,
    ) -> list[Json]: ...


# the converter's AllMessageValues and part TypedDicts are not our JSON shape
CONVERT: Convert = convert_history  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L691-L696


class FakeHandler:
    def __init__(self, script: Metadata, requests: list[Json]) -> None:
        self.script = script
        self.requests = requests

    def get(self, url: str, headers: dict[str, str] | None) -> httpx.Response:
        authorization: str | None = None
        if headers is not None:
            authorization = headers["Authorization"]
        self.requests.append({"url": url, "authorization": authorization})
        if self.script["connect"]:
            message = "the case scripts a metadata connection failure"
            raise httpx.ConnectError(message)
        body = self.script["body"]
        if body is not None:
            return httpx.Response(self.script["status"], text=body)
        return httpx.Response(self.script["status"], json=self.script["json"])


class FakeVertexBase:
    def __init__(self, mints: list[Json], *, fail: bool) -> None:
        self.mints = mints
        self.fail = fail

    def get_access_token(self, credentials: Json, project_id: Json) -> tuple[str, str]:
        # the stub answers every credential alike; only the project matters here
        del credentials
        self.mints.append(project_id)
        if self.fail:
            message = "the case scripts a failing metadata token mint"
            raise RuntimeError(message)
        return ("oracle-token", "project")


def _install(
    script: Metadata, requests: list[Json], mints: list[Json], *, mint_error: bool
) -> None:
    transformation._GCS_METADATA_VERTEX_BASE = FakeVertexBase(mints, fail=mint_error)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L69
    transformation._GCS_METADATA_HTTP_HANDLER = FakeHandler(script, requests)  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L72


# compare_helper drops the error class and message, so a failure travels in the compared value
def _failure(error: BadRequestError, requests: list[Json], mints: list[Json]) -> JsonObject:
    status = error.status_code
    failure: JsonObject = {"status": status, "type": None, "param": None, "code": str(status)}
    return {"requests": requests, "mints": mints, "contents": None, "failure": failure}


def run(input_value: Case) -> JsonObject:
    requests: list[Json] = []
    mints: list[Json] = []
    _install(input_value["metadata"], requests, mints, mint_error=input_value["mint_error"])
    try:
        contents = CONVERT(
            input_value["messages"],
            model=input_value["model"],
            litellm_params=input_value["litellm_params"],
        )
    except BadRequestError as error:
        return _failure(error, requests, mints)
    return {"requests": requests, "mints": mints, "contents": contents, "failure": None}
