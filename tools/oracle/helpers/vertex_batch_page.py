from typing import Final, NotRequired, TypedDict

import httpx
from litellm.llms.vertex_ai.batches.handler import (
    _vertex_batch_list_payload,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/batches/handler.py#L79-L80
)
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

STATUS_OK: Final[int] = 200


class Input(TypedDict):
    body: str


class Job(TypedDict):
    name: NotRequired[str]
    state: NotRequired[str | None]
    createTime: NotRequired[str]


class Page(TypedDict):
    batchPredictionJobs: NotRequired[list[Job] | None]
    nextPageToken: NotRequired[str]


# both sides project the decoded page to each job's name, state and createTime and to the token
def run(input_value: Input) -> Json:
    response = httpx.Response(STATUS_OK, content=input_value["body"].encode())
    payload = _vertex_batch_list_payload(response)
    page = TypeAdapter(Page).validate_python(payload)
    projected: dict[str, Json] = {
        "batchPredictionJobs": TypeAdapter(Json).validate_python(page.get("batchPredictionJobs"))
    }
    token = page.get("nextPageToken")
    if token is not None:
        projected["nextPageToken"] = token
    return projected
