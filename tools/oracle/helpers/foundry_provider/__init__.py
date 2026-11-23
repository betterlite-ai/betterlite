import litellm
from openai import OpenAIError
from pydantic import JsonValue

from .bindings import (
    CHAT_MODEL,
    CLAUDE_MODEL,
    EMBED_MODEL,
    EMPTY,
    MAIN,
    MESSAGES,
    PUBLIC,
    WIRE,
    EmbeddingInput,
    Input,
    NativeBindings,
    Observation,
    Recorder,
    environment,
    frozen_context,
    request_context,
)

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True


def run_chat(recorder: Recorder) -> None:
    try:
        PUBLIC.completion(model=CHAT_MODEL, messages=MESSAGES, custom_llm_provider="azure_ai")
    except ValueError:
        recorder.failed = True
    except OpenAIError:
        recorder.failed = recorder.branch is None


def run_embedding(supplied: EmbeddingInput, recorder: Recorder) -> None:
    try:
        PUBLIC.embedding(
            model=EMBED_MODEL, input=["hello"], custom_llm_provider="azure_ai",
            api_base=supplied["api_base"], api_key=supplied["api_key"],
        )
    except ValueError:
        recorder.failed = True


def observe(supplied: Input) -> Observation:
    recorder = Recorder()
    if supplied["op"] == "rewrite":
        with environment(EMPTY, {}), NativeBindings(MAIN).install(
            frozen_context(CLAUDE_MODEL, supplied["base"], recorder), recorder,
        ):
            run_chat(recorder)
    elif supplied["op"] == "embedding":
        with (
            environment(supplied["globals"], supplied["environment"]),
            NativeBindings(MAIN).install(frozen_context(EMBED_MODEL, None, recorder), recorder),
        ):
            run_embedding(supplied, recorder)
    else:
        recorder.status = supplied["status"]
        with (
            environment(supplied["globals"], supplied["environment"]),
            NativeBindings(MAIN).install(request_context(supplied, recorder), recorder),
        ):
            run_chat(recorder)
    return Observation(
        branch=recorder.branch, base=recorder.base, key=recorder.key,
        headers=recorder.headers, messages=recorder.messages,
        post_calls=recorder.post_calls, errors=recorder.failed,
    )


def run(input_value: dict[str, list[Input]]) -> JsonValue:
    answers = [observe(supplied) for supplied in input_value["inputs"]]
    return WIRE.validate_python(answers)
