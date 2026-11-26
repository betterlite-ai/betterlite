from datetime import datetime
from typing import TypedDict

import httpx
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    vertex_passthrough_logging_handler as handler_module,
)
from pydantic import JsonValue

from b3633 import OUTCOME, call_log_value, kwargs_value, make_logging, result_value

Handler = handler_module.VertexPassthroughLoggingHandler
FIXED_ID = "b3641-id"


class Input(TypedDict):
    status_code: int
    body: str
    url_route: str
    start_time: str
    request_metadata: dict[str, JsonValue]


def run(input_value: Input) -> dict[str, JsonValue]:
    start = datetime.fromisoformat(input_value["start_time"])
    logging_obj = make_logging(start)
    metadata = input_value["request_metadata"]
    body = input_value["body"]
    reply = httpx.Response(input_value["status_code"], content=body.encode())
    litellm_params: dict[str, JsonValue] = {"metadata": metadata}
    outcome = OUTCOME.validate_python(Handler.batch_prediction_jobs_handler(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/vertex_passthrough_logging_handler.py#L791-L801
        reply,
        logging_obj,
        input_value["url_route"],
        body,
        start,
        start,
        cache_hit=False,
        litellm_params=litellm_params,
    ))
    record = result_value(outcome.result)
    record["id"] = FIXED_ID
    return {
        "result": record,
        "kwargs": kwargs_value(outcome.kwargs, metadata),
        "call_log": call_log_value(logging_obj),
    }
