import os
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from datetime import UTC, datetime

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.passthrough.transformation import (
    AzurePassthroughConfig,
    azure_router_model_in_endpoint,
    foreign_azure_deployment,
    is_azure_body_model_inference_endpoint,
    without_api_version,
)
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, JsonValue as Json

from .schema import (
    ADDITIONAL,
    ENVIRONMENT,
    HEADERS,
    JSON,
    NONSTREAM_URL,
    PASSTHROUGH_CALL_TYPE,
    STREAM_MODEL,
    Input,
    Step,
    terminal_view,
)

RECORDED: list[Json] = []


class LoggingView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    call_type: str


def record_stream_chunk_builder(
    *_args: Json, messages: Json = None, **_kwargs: Json
) -> ModelResponse:
    RECORDED.append(messages)
    return ModelResponse()


STREAM_CHUNK_BUILDER: Callable[..., ModelResponse] = record_stream_chunk_builder


@contextmanager
def azure_environment(api_base: str | None, api_key: str | None) -> Iterator[None]:
    saved = {name: value for name, value in os.environ.items() if name in ENVIRONMENT}
    for name in ENVIRONMENT:
        os.environ.pop(name, None)
    if api_base is not None:
        os.environ["AZURE_API_BASE"] = api_base
    if api_key is not None:
        os.environ["AZURE_API_KEY"] = api_key
    try:
        yield
    finally:
        for name in ENVIRONMENT:
            os.environ.pop(name, None)
        os.environ.update(saved)


def logging_obj_for(model: str) -> Logging:
    start = datetime(2026, 1, 1, tzinfo=UTC)
    return Logging(
        model, None, stream=True, call_type=PASSTHROUGH_CALL_TYPE, start_time=start,
        litellm_call_id="b548", function_id="b548",
    )


def changed_call_type(logging_obj: Logging) -> str | None:
    call_type = LoggingView.model_validate(logging_obj).call_type
    if call_type == PASSTHROUGH_CALL_TYPE:
        return None
    return call_type


def inference_report(steps: list[Step]) -> Json:
    config = AzurePassthroughConfig()
    return {
        "steps": [
            {
                "inference": is_azure_body_model_inference_endpoint(step["endpoint"]),
                "streaming": config.is_streaming_request(step["endpoint"], step["request_data"]),
            }
            for step in steps
        ]
    }


def router_report(steps: list[Step]) -> Json:
    return {
        "steps": [
            {"model": azure_router_model_in_endpoint(step["endpoint"], step["router_models"])}
            for step in steps
        ]
    }


def deployment_report(steps: list[Step]) -> Json:
    return {
        "steps": [
            {
                "deployment": foreign_azure_deployment(
                    step["endpoint"], step["model_group"], step["served_models"].copy
                )
            }
            for step in steps
        ]
    }


def url_step(step: Step) -> Json:
    params = step.get("litellm_params")
    litellm_params: dict[str, Json] = {} if params is None else params
    with azure_environment(step.get("env_base"), None):
        url, base = AzurePassthroughConfig().get_complete_url(
            step.get("api_base"),
            None,
            step["model"],
            step["endpoint"],
            step.get("request_query_params"),
            litellm_params,
        )
    return {"url": str(url), "base": base}


def removal_report(removals: list[str], steps: list[Step]) -> Json:
    return {
        "removals": [without_api_version(base) for base in removals],
        "steps": [url_step(step) for step in steps],
    }


def stream_report(steps: list[Step]) -> Json:
    outcomes: list[Json] = []
    for step in steps:
        logging_obj = logging_obj_for(STREAM_MODEL)
        logged = AzurePassthroughConfig().handle_logging_collected_chunks(
            all_chunks=step["chunks"],
            litellm_logging_obj=logging_obj,
            model=STREAM_MODEL,
            custom_llm_provider="azure",
            endpoint=step["endpoint"],
        )
        event: Json = None
        if logged is not None:
            event = terminal_view(logged.model_dump_json())
        outcomes.append({"event": event, "call_type": changed_call_type(logging_obj)})
    return {"steps": outcomes}


def relay_step(step: Step) -> Json:
    RECORDED.clear()
    logging_obj = logging_obj_for(step["model"])
    details = step.get("model_call_details")
    if details is not None:
        logging_obj.model_call_details.update(details)
    logged = AzurePassthroughConfig().handle_logging_collected_chunks(
        all_chunks=step["chunks"],
        litellm_logging_obj=logging_obj,
        model=step["model"],
        custom_llm_provider="azure",
        endpoint=step["endpoint"],
    )
    return {
        "called": bool(RECORDED),
        "result": logged is not None,
        "model": step["model"],
        "messages": RECORDED[0] if RECORDED else None,
        "call_type": changed_call_type(logging_obj),
    }


def relay_report(steps: list[Step]) -> Json:
    original = litellm.stream_chunk_builder
    litellm.stream_chunk_builder = STREAM_CHUNK_BUILDER
    try:
        outcomes = [relay_step(step) for step in steps]
    finally:
        litellm.stream_chunk_builder = original
    return {"steps": outcomes}


def nonstream_step(step: Step) -> Json:
    logging_obj = logging_obj_for(step["model"])
    response = httpx.Response(
        status_code=step["status"],
        headers=step["headers"],
        json=step["body"],
        request=httpx.Request("POST", NONSTREAM_URL),
    )
    logged = AzurePassthroughConfig().logging_non_streaming_response(
        model=step["model"],
        custom_llm_provider="azure",
        httpx_response=response,
        request_data=step["request_data"],
        logging_obj=logging_obj,
        endpoint=step["endpoint"],
    )
    details: dict[str, Json] = logging_obj.model_call_details
    additional = ADDITIONAL.validate_python(details.get("additional_args"))
    complete: Json = None if additional is None else additional.get("complete_input_dict")
    return {
        "result_some": logged is not None,
        "call_type": changed_call_type(logging_obj),
        "logging_input": JSON.validate_python(details.get("input")),
        "complete_input_dict": complete,
    }


def key_step(step: Step) -> Json:
    with azure_environment(None, step.get("env_key")):
        return AzurePassthroughConfig.get_api_key(step.get("api_key"))


def validation_step(step: Step) -> Json:
    saved_api_key = litellm.api_key
    saved_azure_key = litellm.azure_key
    litellm.api_key = step.get("configured_key")
    litellm.azure_key = None
    raw_headers = step.get("headers")
    headers: dict[str, str] = {} if raw_headers is None else dict(raw_headers)
    try:
        with azure_environment(None, None):
            validated = AzurePassthroughConfig().validate_environment(
                headers=headers,
                model="",
                messages=[],
                optional_params={},
                litellm_params={"api_key": step.get("params_key")},
                api_key=step.get("api_key"),
            )
    finally:
        litellm.api_key = saved_api_key
        litellm.azure_key = saved_azure_key
    return HEADERS.validate_python(validated)


def auth_report(key_steps: list[Step], validation_steps: list[Step]) -> Json:
    api_keys: list[Json] = [key_step(step) for step in key_steps]
    headers: list[Json] = [validation_step(step) for step in validation_steps]
    return {"api_keys": api_keys, "headers": headers}


def run(input_value: Input) -> Json:
    function = input_value["function"]
    if function == "api_version_removal":
        report = removal_report(input_value["removals"], input_value["url_steps"])
    elif function == "passthrough_auth_inputs":
        report = auth_report(input_value["key_steps"], input_value["validation_steps"])
    elif function == "body_model_inference_endpoint":
        report = inference_report(input_value["inference_steps"])
    elif function == "router_model_in_path":
        report = router_report(input_value["router_steps"])
    elif function == "foreign_deployment":
        report = deployment_report(input_value["deployment_steps"])
    elif function == "responses_stream_log":
        report = stream_report(input_value["stream_steps"])
    elif function == "messages_relay_log":
        report = relay_report(input_value["relay_steps"])
    else:
        report = {"steps": [nonstream_step(step) for step in input_value["nonstream_steps"]]}
    return report
