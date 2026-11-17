import os
from typing import TypedDict

import litellm
from litellm.llms.vertex_ai.files.handler import VertexAIFilesHandler
from pydantic import JsonValue

litellm.suppress_debug_info = True

READ_ENVIRONMENT = ("GCS_BATCH_BUCKET_NAME", "GCS_BUCKET_NAME", "GCS_PATH_SERVICE_ACCOUNT")


class Input(TypedDict):
    params: dict[str, JsonValue]
    argument: str | dict[str, str] | None
    environment: dict[str, str]


def run(input_value: Input) -> dict[str, str | None]:
    for name in READ_ENVIRONMENT:
        os.environ.pop(name, None)
    for name, value in input_value["environment"].items():
        os.environ[name] = value
    handler = VertexAIFilesHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/files/handler.py#L40
    bucket_name, service_account = handler._resolve_read_gcs_config(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/files/handler.py#L46-L77
        litellm_params=input_value["params"],
        vertex_credentials=input_value["argument"],
    )
    return {"bucket_name": bucket_name, "path_service_account": service_account}
