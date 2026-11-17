from litellm.llms.azure_ai.agents.handler import AzureAIAgentsHandler as AgentHandler

from .answers import (
    annotation_answer,
    extraction_answer,
    reply_answer,
    request_answer,
    stream_answer,
)
from .types import Input, JsonObject


def run(argument: Input) -> JsonObject:
    handler = AgentHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L122-L123
    answer: JsonObject = {}
    if "extractions" in argument:
        answer["extractions"] = [
            extraction_answer(handler, call) for call in argument["extractions"]
        ]
    if "annotations" in argument:
        answer["annotations"] = [
            annotation_answer(handler, call) for call in argument["annotations"]
        ]
    if "replies" in argument:
        answer["replies"] = [reply_answer(handler, call) for call in argument["replies"]]
    if "streams" in argument:
        answer["streams"] = [stream_answer(handler, call) for call in argument["streams"]]
    if "requests" in argument:
        answer["requests"] = [request_answer(handler, call) for call in argument["requests"]]
    return answer
