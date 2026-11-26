from logging import DEBUG, getLogger

import litellm
from pydantic import JsonValue

from b10014.answers import answer
from b10014.case import JSON, Input
from b10014.fakes import Board


def run(input_value: Input) -> JsonValue:
    litellm.vector_store_registry = None
    board = Board()
    logger = getLogger("LiteLLM Proxy")
    logger.setLevel(DEBUG)
    logger.handlers = [board]
    answers: list[JsonValue] = [
        JSON.validate_python(answer(call, board)) for call in input_value["calls"]
    ]
    return JSON.validate_python(answers)
