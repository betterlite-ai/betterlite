import asyncio
import inspect
from collections.abc import Awaitable, Coroutine
from contextlib import closing
from types import ModuleType
from typing import Final

import pytest
from endpoint_server import GOOD_KEY, SPEECH_BYTES, EndpointServer

import betterlite
import betterlite.main
from betterlite._audio.binary import HttpxBinaryResponseContent
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._text import TextCompletionStreamWrapper
from betterlite.main import (
    aembedding,
    aimage_edit,
    aimage_generation,
    aimage_variation,
    amoderation,
    arerank,
    aspeech,
    atext_completion,
    atranscription,
    embedding,
    image_edit,
    image_generation,
    image_variation,
    moderation,
    rerank,
    speech,
    text_completion,
    transcription,
)
from betterlite.types.rerank import RerankResponse
from betterlite.types.utils import EmbeddingResponse, TextCompletionResponse

API_BASE: Final = "/v1"
RERANK_PATH: Final = "/v1/rerank"
INPUT: Final = "hello"
PROMPT: Final = "a surface prompt"
QUERY: Final = "who wrote this"
VOICE: Final = "alloy"
COHERE: Final = "cohere"
EMBEDDING_MODEL: Final = "text-embedding-3-small"
TEXT_MODEL: Final = "gpt-3.5-turbo-instruct"
IMAGE_MODEL: Final = "dall-e-2"
RERANK_MODEL: Final = "rerank-english-v3.0"
WHISPER_MODEL: Final = "whisper-1"
TTS_MODEL: Final = "tts-1"
TRANSCRIPT: Final = "hello"
TEXT_ID: Final = "cmpl-1"
MODERATION_ID: Final = "modr-1"
RERANK_ID: Final = "rerank-1"
GENERATED_URL: Final = "https://example.test/generated.png"
VARIED_URL: Final = "https://example.test/variation.png"
EDITED_URL: Final = "https://example.test/edited.png"
RERANK_SCORE: Final = 0.9
CREATED: Final = 1700000000
EMBEDDING_COUNT: Final = 1
RERANK_COUNT: Final = 1
REFUSED_CLIENT: Final = object()
ASYNC_ANSWER: Final = "a synchronous call answered an awaitable"
STREAM_ANSWER: Final = "a buffered call answered a stream"
IMAGE_BYTES: Final = b"\x89PNG\r\n\x1a\nsurface"
AUDIO_BYTES: Final = (
    b"RIFF\x25\x00\x00\x00WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00"
    b"\x40\x1f\x00\x00\x40\x1f\x00\x00\x01\x00\x08\x00data\x01\x00\x00\x00\x80"
)
CALL_PATHS: Final[tuple[str, ...]] = (
    "/v1/embeddings",
    "/v1/completions",
    "/v1/moderations",
    "/v1/images/generations",
    "/v1/images/variations",
    "/v1/images/edits",
    "/v1/rerank",
    "/v1/audio/transcriptions",
    "/v1/audio/speech",
)
SURFACE_NAMES: Final[tuple[str, ...]] = (
    "embedding", "aembedding", "text_completion", "atext_completion", "moderation",
    "amoderation", "image_generation", "aimage_generation", "image_variation",
    "aimage_variation", "image_edit", "aimage_edit", "rerank", "arerank", "transcription",
    "atranscription", "speech", "aspeech", "TextCompletionStreamWrapper",
)
FACADES: Final[tuple[ModuleType, ...]] = (betterlite, betterlite.main)


def test_surface_names_on_both() -> None:
    for facade in FACADES:
        missing = [name for name in SURFACE_NAMES if name not in dir(facade)]
        assert missing == [], missing


def test_names_are_coroutines() -> None:
    # both facades re-export the same family objects, so main's are the ones to inspect
    assert inspect.iscoroutinefunction(aembedding)
    assert inspect.iscoroutinefunction(atext_completion)
    assert inspect.iscoroutinefunction(amoderation)
    assert inspect.iscoroutinefunction(aimage_generation)
    assert inspect.iscoroutinefunction(aimage_variation)
    assert inspect.iscoroutinefunction(aimage_edit)
    assert inspect.iscoroutinefunction(arerank)
    assert inspect.iscoroutinefunction(atranscription)
    assert inspect.iscoroutinefunction(aspeech)


def test_calls_share_one_process() -> None:
    with closing(EndpointServer()) as server:
        base = server.url + API_BASE
        vectors = embedding(
            model=EMBEDDING_MODEL, input=INPUT, api_base=base, api_key=GOOD_KEY,
        )
        text = text_completion(
            model=TEXT_MODEL, prompt=INPUT, api_base=base, api_key=GOOD_KEY,
        )
        answer = moderation(input=INPUT, api_base=base, api_key=GOOD_KEY)
        generated = image_generation(
            prompt=PROMPT, model=IMAGE_MODEL, api_base=base, api_key=GOOD_KEY,
        )
        varied = image_variation(image=IMAGE_BYTES, api_base=base, api_key=GOOD_KEY)
        edited = image_edit(
            image=IMAGE_BYTES, prompt=PROMPT, model=IMAGE_MODEL,
            api_base=base, api_key=GOOD_KEY,
        )
        ranked = rerank(
            model=RERANK_MODEL, query=QUERY, documents=[QUERY],
            custom_llm_provider=COHERE, api_base=server.url + RERANK_PATH,
            api_key=GOOD_KEY,
        )
        heard = transcription(
            model=WHISPER_MODEL, file=AUDIO_BYTES, api_base=base, api_key=GOOD_KEY,
        )
        spoken = speech(
            model=TTS_MODEL, input=INPUT, voice=VOICE, api_base=base, api_key=GOOD_KEY,
        )
        paths = [call["path"] for call in server.calls]
    assert paths == list(CALL_PATHS)
    match vectors:
        case EmbeddingResponse():
            assert vectors.model == EMBEDDING_MODEL
            assert len(vectors.data) == EMBEDDING_COUNT
        case Awaitable():
            raise AssertionError(ASYNC_ANSWER)
    match text:
        case TextCompletionResponse():
            assert text.id == TEXT_ID
        case TextCompletionStreamWrapper():
            raise AssertionError(STREAM_ANSWER)
    assert answer.id == MODERATION_ID
    assert answer.results[0].flagged is False
    assert generated.created == CREATED
    assert generated.data[0].url == GENERATED_URL
    assert varied.data[0].url == VARIED_URL
    assert edited.data[0].url == EDITED_URL
    match ranked:
        case RerankResponse():
            assert ranked.id == RERANK_ID
            results = ranked.results
            assert results is not None
            assert len(results) == RERANK_COUNT
            assert results[0]["relevance_score"] == RERANK_SCORE
        case Awaitable():
            raise AssertionError(ASYNC_ANSWER)
    assert heard.text == TRANSCRIPT
    match spoken:
        case HttpxBinaryResponseContent():
            assert spoken.content == SPEECH_BYTES
        case Coroutine():
            raise AssertionError(ASYNC_ANSWER)


def test_client_refused_sync() -> None:
    with pytest.raises(UnsupportedCallArgument) as caught:
        embedding(model=EMBEDDING_MODEL, input=INPUT, client=REFUSED_CLIENT)
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        text_completion(model=TEXT_MODEL, prompt=INPUT, client=REFUSED_CLIENT)
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        moderation(input=INPUT, client=REFUSED_CLIENT)
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        image_generation(prompt=PROMPT, model=IMAGE_MODEL, client=REFUSED_CLIENT)
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        image_variation(image=IMAGE_BYTES, client=REFUSED_CLIENT)
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        image_edit(
            image=IMAGE_BYTES, prompt=PROMPT, model=IMAGE_MODEL, client=REFUSED_CLIENT,
        )
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        rerank(
            model=RERANK_MODEL, query=QUERY, documents=[QUERY],
            custom_llm_provider=COHERE, client=REFUSED_CLIENT,
        )
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        transcription(model=WHISPER_MODEL, file=AUDIO_BYTES, client=REFUSED_CLIENT)
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        speech(model=TTS_MODEL, input=INPUT, voice=VOICE, client=REFUSED_CLIENT)
    assert caught.value.argument == "client"


def test_client_refused_async() -> None:
    with pytest.raises(UnsupportedCallArgument) as caught:
        asyncio.run(aembedding(model=EMBEDDING_MODEL, input=INPUT, client=REFUSED_CLIENT))
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        asyncio.run(atext_completion(model=TEXT_MODEL, prompt=INPUT, client=REFUSED_CLIENT))
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        asyncio.run(amoderation(input=INPUT, client=REFUSED_CLIENT))
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        asyncio.run(aimage_generation(model=IMAGE_MODEL, prompt=PROMPT, client=REFUSED_CLIENT))
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        asyncio.run(aimage_variation(image=IMAGE_BYTES, client=REFUSED_CLIENT))
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        asyncio.run(
            aimage_edit(
                image=IMAGE_BYTES, model=IMAGE_MODEL, prompt=PROMPT, client=REFUSED_CLIENT,
            )
        )
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        asyncio.run(
            arerank(
                model=RERANK_MODEL, query=QUERY, documents=[QUERY],
                custom_llm_provider=COHERE, client=REFUSED_CLIENT,
            )
        )
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        asyncio.run(atranscription(model=WHISPER_MODEL, file=AUDIO_BYTES, client=REFUSED_CLIENT))
    assert caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as caught:
        asyncio.run(aspeech(model=TTS_MODEL, input=INPUT, voice=VOICE, client=REFUSED_CLIENT))
    assert caught.value.argument == "client"
