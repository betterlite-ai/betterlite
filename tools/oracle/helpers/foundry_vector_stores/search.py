import asyncio

from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import JsonValue

from foundry_vector_stores.inputs import (
    OptionalParams,
    SearchQueryFieldsInput,
    SearchScenario,
    VectorQueryWithEmbeddingInput,
)
from foundry_vector_stores.surface import CONFIG, JSON, Fixture, Logged, logging_fixture


def request_outcome(scenario: SearchScenario, logging: Logging) -> dict[str, JsonValue]:
    vector = [0.25, 0.5]
    embedding = scenario.get("embedding")
    if embedding is not None:
        vector = embedding
    params = scenario.get("optional_params")
    if params is None:
        params = OptionalParams()
    extra_body = scenario.get("extra_body")
    executor = Fixture([vector])
    try:
        if scenario.get("asynchronous"):
            url, body = asyncio.run(
                CONFIG.atransform_search_vector_store_request(
                    vector_store_id=scenario["vector_store_id"],
                    query=scenario["query"],
                    vector_store_search_optional_params=params,
                    api_base=scenario["api_base"],
                    litellm_logging_obj=logging,
                    litellm_params=scenario["litellm_params"],
                    extra_body=extra_body,
                    embedding_executor=executor,
                )
            )
        else:
            url, body = CONFIG.transform_search_vector_store_request(
                vector_store_id=scenario["vector_store_id"],
                query=scenario["query"],
                vector_store_search_optional_params=params,
                api_base=scenario["api_base"],
                litellm_logging_obj=logging,
                litellm_params=scenario["litellm_params"],
                extra_body=extra_body,
                embedding_executor=executor,
            )
    except (ValueError, IndexError, KeyError):
        return {
            "ok": False,
            "status": None,
            "url": None,
            "body": None,
            "calls": executor.calls,
            "extra_body": extra_body,
        }
    details = Logged.model_validate(logging).model_call_details
    return {
        "ok": True,
        "url": url,
        "body": JSON.validate_python(body),
        "logging": JSON.validate_json(details.model_dump_json()),
        "calls": executor.calls,
        "extra_body": extra_body,
    }


def search_report(input_value: SearchQueryFieldsInput) -> JsonValue:
    searches: list[JsonValue] = [
        request_outcome(scenario, logging_fixture()) for scenario in input_value["searches"]
    ]
    return {"searches": searches}


def embedded_report(input_value: VectorQueryWithEmbeddingInput) -> JsonValue:
    scenarios: list[JsonValue] = [
        {
            key: value
            for key, value in request_outcome(scenario, logging_fixture()).items()
            if key not in {"logging", "extra_body"}
        }
        for scenario in input_value["scenarios"]
    ]
    return {"scenarios": scenarios}
