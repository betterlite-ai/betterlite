from typing import TypedDict

from fastapi import FastAPI
from fastapi.testclient import TestClient
from litellm.proxy.analytics_endpoints.analytics_endpoints import router
from pydantic import JsonValue, TypeAdapter

PATHS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])

app = FastAPI()
app.include_router(router)


class Input(TypedDict):
    path: str


def run(input_value: Input) -> JsonValue:
    response = TestClient(app).get("/openapi.json")
    document = PATHS.validate_json(response.text)
    paths = PATHS.validate_python(document["paths"])
    answer: dict[str, JsonValue] = {
        "path": input_value["path"],
        "documented": input_value["path"] in paths,
    }
    return answer
