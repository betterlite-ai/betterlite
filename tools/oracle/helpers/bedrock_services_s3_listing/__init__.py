import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1459-L1488
import litellm
from litellm.llms.bedrock.files.transformation import BedrockFilesConfig
from pydantic import JsonValue

from .calls import SIGNING_SECONDS, FixedNow, reply, signed_request
from .responses import delete_response, list_response, retrieve
from .schema import Input

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    botocore.auth.get_current_datetime = FixedNow(SIGNING_SECONDS)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1459-L1488
    config = BedrockFilesConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L392-L394
    results: list[JsonValue] = []
    if input_value["operation"] == "list_request":
        results = [signed_request(config, "list_request", call) for call in input_value["requests"]]
    elif input_value["operation"] == "next_request":
        results = [signed_request(config, "next_request", call) for call in input_value["next"]]
    elif input_value["operation"] == "list_response":
        results = [list_response(config, call) for call in input_value["listings"]]
    elif input_value["operation"] == "delete_request":
        results = [signed_request(config, "delete_request", call) for call in input_value["deletes"]]
    elif input_value["operation"] == "content_request":
        results = [signed_request(config, "content_request", call) for call in input_value["contents"]]
    elif input_value["operation"] == "delete_response":
        results = [delete_response(config, call) for call in input_value["delete_responses"]]
    else:
        results = [
            entry
            for call in input_value["retrieves"]
            for entry in retrieve(config, call)
        ]
    return reply({"results": results})
