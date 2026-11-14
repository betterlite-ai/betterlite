from typing import TypedDict

import litellm
from litellm.llms.vertex_ai.batches.handler import VertexAIBatchPrediction
from pydantic import JsonValue

litellm.suppress_debug_info = True


class Input(TypedDict):
    api_base: str | None
    project: str
    location: str
    model: str


def run(input_value: Input) -> JsonValue:
    batch = VertexAIBatchPrediction(gcs_bucket_name="bucket")
    check, build = batch._check_custom_proxy, batch._build_endpoint_resolution_url  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/batches/handler.py#L182
    default_url = batch.create_vertex_batch_url(
        vertex_location=input_value["location"],
        vertex_project=input_value["project"],
    )
    endpoint = default_url.split(":")[-1] if ":" in default_url else ""
    _, collection = check(
        api_base=input_value["api_base"],
        custom_llm_provider="vertex_ai",
        gemini_api_key=None,
        endpoint=endpoint,
        stream=None,
        auth_header=None,
        url=default_url,
        model=None,
        vertex_project=input_value["project"],
        vertex_location=input_value["location"],
        vertex_api_version="v1",
    )
    return {
        "collection": collection,
        "endpoint": build(input_value["api_base"], input_value["model"], input_value["location"]),
    }
