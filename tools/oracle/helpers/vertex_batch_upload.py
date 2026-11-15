from typing import TypedDict, cast

from litellm.llms.base_llm.files.transformation import BaseFileUploadStream
from litellm.llms.vertex_ai.files.transformation import VertexAIFilesConfig
from litellm.types.llms.openai import CreateFileRequest
from pydantic import JsonValue


class UploadPlan(TypedDict):
    body_stream: BaseFileUploadStream
    content_type: str


class Call(TypedDict):
    content: str
    passthrough: JsonValue


class Input(TypedDict):
    calls: list[Call]


def answer(call: Call) -> JsonValue:
    create_file_data: CreateFileRequest = {
        "file": ("batch.jsonl", call["content"].encode("utf-8"), "application/jsonl"),
        "purpose": "batch",
    }
    planned = VertexAIFilesConfig().transform_create_file_request(
        "",
        create_file_data,
        {},
        {"passthrough": call["passthrough"]},
    )
    plan = cast(dict[str, UploadPlan], planned)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/files/transformation.py#L1055-L1059
    stream = plan["streaming_media_upload"]["body_stream"]
    chunks: list[JsonValue] = [chunk.decode("utf-8") for chunk in stream.iter_bytes()]
    return {"chunks": chunks}


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return answers
