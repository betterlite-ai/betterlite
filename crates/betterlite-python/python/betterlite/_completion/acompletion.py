from collections.abc import Awaitable

from pydantic import JsonValue

from betterlite._completion.arguments import CallValue, SessionLike, TimeoutValues
from betterlite._completion.completion import (
    call_arguments_from,
    completion,
    completion_from_arguments,
    settle_outcome,
)
from betterlite._completion.stream import CompletionOutcome
from betterlite.llms.base_llm.base_utils import ResponseFormatModel
from betterlite.types.utils import Message


async def acompletion(
    model: str | None,
    messages: list[dict[str, JsonValue] | Message] = [],
    functions: JsonValue = None, function_call: JsonValue = None,
    timeout: float | str | TimeoutValues | None = None,
    temperature: JsonValue = None, top_p: JsonValue = None, n: JsonValue = None,
    stream: JsonValue = None, stream_options: JsonValue = None, stop: JsonValue = None,
    max_tokens: JsonValue = None, max_completion_tokens: JsonValue = None,
    modalities: JsonValue = None, prediction: JsonValue = None, audio: JsonValue = None,
    presence_penalty: JsonValue = None, frequency_penalty: JsonValue = None,
    logit_bias: JsonValue = None, user: JsonValue = None,
    response_format: dict[str, JsonValue] | type[ResponseFormatModel] | None = None,
    seed: JsonValue = None, tools: JsonValue = None, tool_choice: JsonValue = None,
    parallel_tool_calls: JsonValue = None, logprobs: JsonValue = None,
    top_logprobs: JsonValue = None, deployment_id: JsonValue = None,
    reasoning_effort: JsonValue = None, verbosity: JsonValue = None,
    safety_identifier: JsonValue = None, service_tier: JsonValue = None,
    store: JsonValue = None, prompt_cache_key: JsonValue = None,
    base_url: JsonValue = None, api_version: JsonValue = None,
    api_key: JsonValue = None, model_list: JsonValue = None,
    extra_headers: JsonValue = None, thinking: JsonValue = None,
    web_search_options: JsonValue = None,
    include_server_side_tool_invocations: JsonValue = None,  # ast-grep-ignore: no-long-names
    shared_session: SessionLike | None = None, enable_json_schema_validation: JsonValue = None,
    **kwargs: CallValue,
) -> CompletionOutcome:
    kwargs.pop("acompletion", None)
    pending: CompletionOutcome | Awaitable[CompletionOutcome] = completion(
        model=model, messages=messages, functions=functions, function_call=function_call,
        timeout=timeout, temperature=temperature, top_p=top_p, n=n, stream=stream,
        stream_options=stream_options, stop=stop, max_tokens=max_tokens,
        max_completion_tokens=max_completion_tokens, modalities=modalities,
        prediction=prediction, audio=audio, presence_penalty=presence_penalty,
        frequency_penalty=frequency_penalty, logit_bias=logit_bias, user=user,
        response_format=response_format, seed=seed, tools=tools, tool_choice=tool_choice,
        parallel_tool_calls=parallel_tool_calls, logprobs=logprobs,
        top_logprobs=top_logprobs, deployment_id=deployment_id,
        reasoning_effort=reasoning_effort, verbosity=verbosity,
        safety_identifier=safety_identifier, service_tier=service_tier, store=store,
        prompt_cache_key=prompt_cache_key, base_url=base_url, api_version=api_version,
        api_key=api_key, model_list=model_list, extra_headers=extra_headers,
        thinking=thinking, web_search_options=web_search_options,
        # ast-grep-ignore: no-long-names
        include_server_side_tool_invocations=include_server_side_tool_invocations,
        shared_session=shared_session,
        enable_json_schema_validation=enable_json_schema_validation,
        acompletion=True, **kwargs,
    )
    return await settle_outcome(pending)


def acreate(*args: JsonValue, **kwargs: CallValue) -> Awaitable[CompletionOutcome]:
    # litellm.utils.acreate: a thin client that hands its arguments to acompletion
    arguments = call_arguments_from(args, kwargs)
    return settle_outcome(completion_from_arguments({**arguments, "acompletion": True}))
