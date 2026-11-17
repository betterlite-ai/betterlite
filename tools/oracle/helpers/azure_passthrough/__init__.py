import os
from collections.abc import Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from typing import Final, Protocol, cast

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
from pydantic import BaseModel, ConfigDict
from pydantic import JsonValue as Json

from . import steps

RECORDED: list[Json] = []


class Dumpable(Protocol):
    def model_dump_json(self) -> str: ...


class AzurePassthroughConfigView(Protocol):
    def is_streaming_request(self, endpoint: str, request_data: dict[str, Json]) -> bool: ...

    def get_complete_url(
        self, api_base: str | None, api_key: str | None, model: str, endpoint: str,
        request_query_params: dict[str, Json] | None, litellm_params: dict[str, Json],
    ) -> tuple[httpx.URL, str]: ...

    def validate_environment(
        self, headers: dict[str, str], model: str, messages: list[Json],
        optional_params: dict[str, Json], litellm_params: dict[str, Json],
        api_key: str | None = None,
    ) -> dict[str, str]: ...

    def logging_non_streaming_response(
        self, model: str, custom_llm_provider: str, httpx_response: httpx.Response,
        request_data: dict[str, Json], logging_obj: Logging, endpoint: str,
    ) -> Dumpable | None: ...

    def handle_logging_collected_chunks(  # ast-grep-ignore: no-long-names
        self, all_chunks: list[str], litellm_logging_obj: Logging, model: str,
        custom_llm_provider: str, endpoint: str,
    ) -> Dumpable | None: ...


CONFIG: Final[AzurePassthroughConfigView] = cast(AzurePassthroughConfigView, AzurePassthroughConfig())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/passthrough/transformation.py#L105-L231


class LoggingView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    call_type: str


def record_stream_chunk_builder(
    *_args: object, messages: object = None, **_kwargs: object
) -> ModelResponse:
    # parameters take the top type so the stand-in is assignable to LiteLLM's own signature
    RECORDED.append(steps.JSON.validate_python(messages))
    return ModelResponse()


@contextmanager
def azure_environment(api_base: str | None, api_key: str | None) -> Generator[None]:
    saved = {name: os.environ.pop(name) for name in steps.ENVIRONMENT if name in os.environ}
    try:
        if api_base is not None:
            os.environ["AZURE_API_BASE"] = api_base
        if api_key is not None:
            os.environ["AZURE_API_KEY"] = api_key
        yield
    finally:
        for name in steps.ENVIRONMENT:
            os.environ.pop(name, None)
        os.environ.update(saved)


def logging_obj_for(model: str) -> Logging:
    start = datetime(2026, 1, 1, tzinfo=UTC)
    return Logging(
        model, None, stream=True, call_type=steps.PASSTHROUGH_CALL_TYPE, start_time=start,
        litellm_call_id="b548", function_id="b548",
    )


def changed_call_type(logging_obj: Logging) -> str | None:
    call_type = LoggingView.model_validate(logging_obj).call_type
    return None if call_type == steps.PASSTHROUGH_CALL_TYPE else call_type


def inference_report(payload: dict[str, Json]) -> Json:
    return {
        "steps": [
            {
                "inference": is_azure_body_model_inference_endpoint(step.endpoint),
                "streaming": CONFIG.is_streaming_request(step.endpoint, step.request_data),
            }
            for step in steps.INFERENCE_STEPS.validate_python(payload["inference_steps"])
        ]
    }


def router_report(payload: dict[str, Json]) -> Json:
    return {
        "steps": [
            {"model": azure_router_model_in_endpoint(step.endpoint, step.router_models)}
            for step in steps.ROUTER_STEPS.validate_python(payload["router_steps"])
        ]
    }


def deployment_report(payload: dict[str, Json]) -> Json:
    return {
        "steps": [
            {
                "deployment": foreign_azure_deployment(
                    step.endpoint, step.model_group, step.served_models.copy
                )
            }
            for step in steps.DEPLOYMENT_STEPS.validate_python(payload["deployment_steps"])
        ]
    }


def url_step(step: steps.UrlStep) -> Json:
    with azure_environment(step.env_base, None):
        url, base = CONFIG.get_complete_url(
            step.api_base, None, step.model, step.endpoint, step.request_query_params,
            step.litellm_params,
        )
    return {"url": str(url), "base": base}


def removal_report(payload: dict[str, Json]) -> Json:
    removals = steps.REMOVALS.validate_python(payload["removals"])
    url_steps = steps.URL_STEPS.validate_python(payload["url_steps"])
    return {
        "removals": [without_api_version(base) for base in removals],
        "steps": [url_step(step) for step in url_steps],
    }


def stream_report(payload: dict[str, Json]) -> Json:
    outcomes: list[Json] = []
    for step in steps.STREAM_STEPS.validate_python(payload["stream_steps"]):
        logging_obj = logging_obj_for(steps.STREAM_MODEL)
        logged = CONFIG.handle_logging_collected_chunks(
            all_chunks=step.chunks,
            litellm_logging_obj=logging_obj,
            model=steps.STREAM_MODEL,
            custom_llm_provider="azure",
            endpoint=step.endpoint,
        )
        event: Json = None if logged is None else steps.terminal_view(logged.model_dump_json())
        outcomes.append({"event": event, "call_type": changed_call_type(logging_obj)})
    return {"steps": outcomes}


def relay_report(payload: dict[str, Json]) -> Json:
    # LiteLLM looks the assembler up on its module at call time, so the recorder covers every step
    litellm.stream_chunk_builder = record_stream_chunk_builder
    outcomes: list[Json] = []
    for step in steps.RELAY_STEPS.validate_python(payload["relay_steps"]):
        RECORDED.clear()
        logging_obj = logging_obj_for(step.model)
        if step.model_call_details is not None:
            details: dict[str, Json] = logging_obj.model_call_details
            details.update(step.model_call_details)
        logged = CONFIG.handle_logging_collected_chunks(
            all_chunks=step.chunks,
            litellm_logging_obj=logging_obj,
            model=step.model,
            custom_llm_provider="azure",
            endpoint=step.endpoint,
        )
        outcomes.append(
            {
                "called": bool(RECORDED),
                "result": logged is not None,
                "model": step.model,
                "messages": RECORDED[0] if RECORDED else None,
                "call_type": changed_call_type(logging_obj),
            }
        )
    return {"steps": outcomes}


def nonstream_report(payload: dict[str, Json]) -> Json:
    outcomes: list[Json] = []
    for step in steps.NONSTREAM_STEPS.validate_python(payload["nonstream_steps"]):
        logging_obj = logging_obj_for(step.model)
        request = httpx.Request("POST", steps.NONSTREAM_URL)
        response = httpx.Response(
            step.status, headers=step.headers, json=step.body, request=request
        )
        logged = CONFIG.logging_non_streaming_response(
            model=step.model,
            custom_llm_provider="azure",
            httpx_response=response,
            request_data=step.request_data,
            logging_obj=logging_obj,
            endpoint=step.endpoint,
        )
        details: dict[str, Json] = logging_obj.model_call_details
        additional = steps.ADDITIONAL.validate_python(details.get("additional_args"))
        complete: Json = None if additional is None else additional.get("complete_input_dict")
        outcomes.append(
            {
                "result_some": logged is not None,
                "call_type": changed_call_type(logging_obj),
                "logging_input": steps.JSON.validate_python(details.get("input")),
                "complete_input_dict": complete,
            }
        )
    return {"steps": outcomes}


def key_step(step: steps.KeyStep) -> Json:
    with azure_environment(None, step.env_key):
        return AzurePassthroughConfig.get_api_key(step.api_key)


def validation_step(step: steps.ValidationStep) -> Json:
    saved_api_key = litellm.api_key
    saved_azure_key = litellm.azure_key
    litellm.api_key = step.configured_key
    litellm.azure_key = None
    try:
        with azure_environment(None, None):
            validated = CONFIG.validate_environment(
                headers=step.headers,
                model="",
                messages=[],
                optional_params={},
                litellm_params={"api_key": step.params_key},
                api_key=step.api_key,
            )
    finally:
        litellm.api_key = saved_api_key
        litellm.azure_key = saved_azure_key
    return steps.JSON.validate_python(validated)


def auth_report(payload: dict[str, Json]) -> Json:
    return {
        "api_keys": [
            key_step(step) for step in steps.KEY_STEPS.validate_python(payload["key_steps"])
        ],
        "headers": [
            validation_step(step)
            for step in steps.VALIDATION_STEPS.validate_python(payload["validation_steps"])
        ],
    }


def run(input_value: dict[str, Json]) -> Json:
    if input_value["function"] == "api_version_removal":
        report = removal_report(input_value)
    elif input_value["function"] == "passthrough_auth_inputs":
        report = auth_report(input_value)
    elif input_value["function"] == "body_model_inference_endpoint":
        report = inference_report(input_value)
    elif input_value["function"] == "router_model_in_path":
        report = router_report(input_value)
    elif input_value["function"] == "foreign_deployment":
        report = deployment_report(input_value)
    elif input_value["function"] == "responses_stream_log":
        report = stream_report(input_value)
    elif input_value["function"] == "messages_relay_log":
        report = relay_report(input_value)
    else:
        report = nonstream_report(input_value)
    return report
