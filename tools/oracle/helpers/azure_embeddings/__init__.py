import asyncio
from collections.abc import Coroutine
from datetime import UTC, datetime

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.azure import AzureChatCompletion
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.types.utils import EmbeddingResponse
from openai import APIError, APIStatusError
from pydantic import JsonValue as Json

from azure_embeddings.fixtures import Session, make_client
from azure_embeddings.report import Recorder, azure_value, reply_value
from azure_embeddings.types import Input, Step

API_BASE = "https://example.openai.azure.com"


def logging_object(recorder: Recorder) -> Logging:
    logging_obj = Logging(
        model="my-deployment",
        messages=None,
        stream=False,
        call_type="embedding",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b534",
        function_id="b534",
    )
    logging_obj.update_environment_variables(
        litellm_params={"logger_fn": recorder.log},
        optional_params={},
        model="my-deployment",
    )
    return logging_obj


def call_embedding(step: Step, session: Session, logging_obj: Logging) -> EmbeddingResponse:
    outcome = AzureChatCompletion().embedding(
        model=step["model"] if "model" in step else "my-deployment",
        input=step["input"],
        api_base=step["api_base"] if "api_base" in step else API_BASE,
        api_version=step["api_version"] if "api_version" in step else "2024-02-01",
        timeout=step["timeout"] if "timeout" in step else 30.0,
        logging_obj=logging_obj,
        model_response=EmbeddingResponse(),
        optional_params=dict(step["optional_params"]) if "optional_params" in step else {},
        api_key=step["api_key"] if "api_key" in step else "azure-key",
        azure_ad_token=step["azure_ad_token"] if "azure_ad_token" in step else None,
        max_retries=step["max_retries"] if "max_retries" in step else None,
        client=make_client(step["client"], session),
        aembedding=step["aembedding"] if "aembedding" in step else False,
        headers=step["headers"] if "headers" in step else None,
        litellm_params=dict(step["litellm_params"]) if "litellm_params" in step else None,
    )
    match outcome:
        case Coroutine():
            return asyncio.run(outcome)
        case EmbeddingResponse():
            return outcome


def raised_value(kind: str, status: int | None, message: str | None) -> dict[str, Json]:
    return {
        "kind": kind,
        "status": status,
        "message": message,
        "headers": None,
    }


def step_value(step: Step) -> dict[str, Json]:
    session = Session(step)
    recorder = Recorder()
    result: dict[str, Json] | None = None
    raised: dict[str, Json] | None = None
    try:
        result = reply_value(call_embedding(step, session, logging_object(recorder)))
    except AzureOpenAIError as error:
        raised = azure_value(error)
    except ValueError as error:
        raised = raised_value("value", None, str(error))
    except APIStatusError as error:
        raised = raised_value("sdk_status", error.status_code, None)
    except APIError:
        raised = raised_value("sdk_other", None, None)
    return {
        "result": result,
        "raised": raised,
        "pre_call": recorder.pre,
        "post_call": recorder.post,
        "calls": session.calls,
    }


def run(input_value: Input) -> dict[str, Json]:
    # an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
    litellm.suppress_debug_info = True
    return {"steps": [step_value(step) for step in input_value["steps"]]}
