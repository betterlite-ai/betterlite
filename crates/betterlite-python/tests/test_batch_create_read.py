import asyncio
import inspect
from collections.abc import Awaitable, Mapping

import pytest
from batch_server import (
    BATCH_ID,
    BATCH_PATH,
    CREATED_AT,
    DEPLOYMENT_ID,
    ENDPOINT,
    GOOD_KEY,
    INPUT_FILE_ID,
    LITELLM,
    METADATA,
    MISSING_ID,
    MODEL,
    MODEL_INFO,
    NOT_FOUND,
    WINDOW,
    BatchServer,
    parameter_rows,
)

from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._native import NativeBatchError
from betterlite.batches._create_retrieve import (
    LiteLLMBatch,
    acreate_batch,
    aretrieve_batch,
    create_batch,
    retrieve_batch,
)
from betterlite.exceptions import NotFoundError
from betterlite.types.utils.base import InitValue


def made_batch(result: LiteLLMBatch | Awaitable[LiteLLMBatch]) -> LiteLLMBatch:
    match result:
        case LiteLLMBatch():
            return result
        case Awaitable():
            raise AssertionError("the call answered with a coroutine")


async def awaited_batch(pending: Awaitable[LiteLLMBatch]) -> LiteLLMBatch:
    batch = await pending
    assert batch.id == BATCH_ID
    return batch


def hidden_of(batch: LiteLLMBatch) -> dict[str, InitValue]:
    params = batch["_hidden_params"]
    match params:
        case Mapping():
            return dict(params)
        case object():
            raise AssertionError("the batch carries no hidden parameters")


def native_cause(error: BaseException) -> NativeBatchError:
    cause = error.__cause__
    match cause:
        case NativeBatchError():
            return cause
        case object():
            raise AssertionError("the exception does not carry the native error as its cause")


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(create_batch)) == parameter_rows(
        inspect.signature(LITELLM.create_batch)
    )
    assert parameter_rows(inspect.signature(acreate_batch)) == parameter_rows(
        inspect.signature(LITELLM.acreate_batch)
    )
    assert parameter_rows(inspect.signature(retrieve_batch)) == parameter_rows(
        inspect.signature(LITELLM.retrieve_batch)
    )
    assert parameter_rows(inspect.signature(aretrieve_batch)) == parameter_rows(
        inspect.signature(LITELLM.aretrieve_batch)
    )
    assert not inspect.iscoroutinefunction(create_batch)
    assert not inspect.iscoroutinefunction(retrieve_batch)
    assert inspect.iscoroutinefunction(acreate_batch)
    assert inspect.iscoroutinefunction(aretrieve_batch)


def test_create_matches_litellm() -> None:
    with BatchServer() as server:
        reference = LITELLM.create_batch(
            completion_window=WINDOW,
            endpoint=ENDPOINT,
            input_file_id=INPUT_FILE_ID,
            metadata=METADATA,
            model=MODEL,
            model_info=MODEL_INFO,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        made = made_batch(
            create_batch(
                completion_window=WINDOW,
                endpoint=ENDPOINT,
                input_file_id=INPUT_FILE_ID,
                metadata=METADATA,
                model=MODEL,
                model_info=MODEL_INFO,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        assert len(server.calls) == 2
        reference_call, ours_call = server.calls
    assert made.model_dump() == reference.model_dump()
    assert made.created_at == CREATED_AT
    assert ours_call["path"] == BATCH_PATH
    assert reference_call["path"] == BATCH_PATH
    assert ours_call["body"].get("completion_window") == WINDOW
    assert ours_call["body"].get("endpoint") == ENDPOINT
    assert ours_call["body"].get("input_file_id") == INPUT_FILE_ID
    assert ours_call["body"].get("metadata") == METADATA
    assert ours_call["body"] == reference_call["body"]
    hidden = hidden_of(made)
    assert hidden["llm_provider"] == "openai"
    assert hidden["model_id"] == DEPLOYMENT_ID
    assert hidden["litellm_model_name"] == MODEL
    assert hidden["api_base"] == server.url


def test_retrieve_matches_litellm() -> None:
    with BatchServer() as server:
        reference = LITELLM.retrieve_batch(
            batch_id=BATCH_ID, api_base=server.url, api_key=GOOD_KEY,
        )
        found = made_batch(
            retrieve_batch(batch_id=BATCH_ID, api_base=server.url, api_key=GOOD_KEY)
        )
        assert len(server.calls) == 2
        reference_call, ours_call = server.calls
    assert found.model_dump() == reference.model_dump()
    assert found.status == "in_progress"
    assert ours_call["path"] == f"{BATCH_PATH}/{BATCH_ID}"
    assert reference_call["path"] == f"{BATCH_PATH}/{BATCH_ID}"
    hidden = hidden_of(found)
    assert hidden["llm_provider"] == "openai"
    assert hidden["model_id"] is None
    assert hidden["litellm_model_name"] is None
    assert hidden["api_base"] == server.url


def test_retrieve_coroutines() -> None:
    with BatchServer() as server:
        flagged_create = create_batch(
            completion_window=WINDOW,
            endpoint=ENDPOINT,
            input_file_id=INPUT_FILE_ID,
            model=MODEL,
            api_base=server.url,
            api_key=GOOD_KEY,
            acreate_batch=True,
        )
        assert inspect.isawaitable(flagged_create)
        created = asyncio.run(awaited_batch(flagged_create))
        assert created.status == "validating"
        flagged_retrieve = retrieve_batch(
            batch_id=BATCH_ID,
            api_base=server.url,
            api_key=GOOD_KEY,
            aretrieve_batch=True,
        )
        assert inspect.isawaitable(flagged_retrieve)
        retrieved = asyncio.run(awaited_batch(flagged_retrieve))
        assert retrieved.status == "in_progress"
        direct_create = acreate_batch(
            completion_window=WINDOW,
            endpoint=ENDPOINT,
            input_file_id=INPUT_FILE_ID,
            model=MODEL,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        assert inspect.isawaitable(direct_create)
        assert asyncio.run(awaited_batch(direct_create)).status == "validating"
        direct_retrieve = aretrieve_batch(
            batch_id=BATCH_ID, api_base=server.url, api_key=GOOD_KEY,
        )
        assert inspect.isawaitable(direct_retrieve)
        assert asyncio.run(awaited_batch(direct_retrieve)).status == "in_progress"


def test_error_classes() -> None:
    with BatchServer() as server:
        with pytest.raises(NotFoundError) as caught:
            retrieve_batch(batch_id=MISSING_ID, api_base=server.url, api_key=GOOD_KEY)
    cause = native_cause(caught.value)
    assert cause is caught.value.__cause__
    assert caught.value.status_code == NOT_FOUND
    assert caught.value.llm_provider == "openai"
    with pytest.raises(UnsupportedCallArgument) as refused:
        create_batch(
            completion_window=WINDOW,
            endpoint=ENDPOINT,
            input_file_id=INPUT_FILE_ID,
            client=object(),
        )
    assert refused.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as session_refused:
        retrieve_batch(batch_id=BATCH_ID, shared_session=object())
    assert session_refused.value.argument == "shared_session"
