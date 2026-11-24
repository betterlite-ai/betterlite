from collections.abc import Mapping

from betterlite._cost.engine import usage_value
from betterlite._cost.objects import dump_object
from betterlite._cost.wire import JsonSource, UsageRequestWire, object_value
from betterlite.types.decisions import DecisionsResponse, OpenAIDecisionResponse
from betterlite.types.utils.base import Dumpable
from betterlite.types.utils.transcription import TranscriptionResponse
from betterlite.types.utils.usage import Usage


# get_usage_object: the response's usage as LiteLLM's Usage model, or None when it carries none.
def get_usage_object(completion_response: JsonSource) -> Usage | None:
    dumped = dump_object(completion_response)
    request = UsageRequestWire(
        kind=usage_kind(completion_response),
        usage=object_value(dumped.get("usage")),
    )
    return usage_value(request)


def usage_kind(completion_response: JsonSource) -> str:
    match completion_response:
        case DecisionsResponse():
            return "decisions_ir"
        case OpenAIDecisionResponse():
            return "decisions_openai"
        case TranscriptionResponse():
            return "transcription"
        case Dumpable() | Mapping():
            return "other"
