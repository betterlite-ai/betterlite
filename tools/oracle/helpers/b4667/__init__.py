import asyncio

from litellm.proxy._experimental.mcp_server.outbound_credentials.adapter import (
    oauth_protected_resource_path,
)
from pydantic import JsonValue

from b4667.answers import spec_answer, to_subject_answer
from b4667.challenges import exchange_challenge_answer, user_challenge_answer
from b4667.resolution import resolver_answer
from b4667.wire import SERVER_ADAPTER, Input


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "to_subject":
            return to_subject_answer(input_value["principal"], input_value["subject_token"])
        case "to_server_spec":
            return spec_answer(SERVER_ADAPTER.validate_python(input_value["server"]))
        case "protected_resource_path":
            return oauth_protected_resource_path(
                input_value["root_path"], SERVER_ADAPTER.validate_python(input_value["server"])
            )
        case "user_oauth_challenge":
            return user_challenge_answer(
                SERVER_ADAPTER.validate_python(input_value["server"]), input_value["root_path"]
            )
        case "token_exchange_challenge":
            return exchange_challenge_answer(
                SERVER_ADAPTER.validate_python(input_value["server"]),
                input_value["root_path"],
                input_value["claims"],
            )
        case "credential_resolver":
            return asyncio.run(resolver_answer(input_value))
