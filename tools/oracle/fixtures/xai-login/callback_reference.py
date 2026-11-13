import http.client
import json
import sys
import threading
from importlib.metadata import version

from litellm.llms.xai.oauth import _CallbackHandler, _CallbackServer

PINNED_COMMIT = "d665dc1a6315847fcad6cc2c1264ceb47c06abce"
PINNED_VERSION = "1.105.1"
EXPECTED_STATE = "state-expected"
CASES = [
    "matching_state_and_code",
    "matching_state_with_provider_error",
    "matching_state_without_code",
    "state_mismatch",
    "duplicate_query_fields_keep_the_first",
    "percent_escaped_values",
    "blank_query_fields_are_dropped",
    "unrelated_path",
]
TARGETS = {
    "matching_state_and_code": "/callback?code=code-1&state=state-expected",
    "matching_state_with_provider_error": "/callback?error=access_denied&error_description=user+denied&state=state-expected",
    "matching_state_without_code": "/callback?state=state-expected",
    "state_mismatch": "/callback?code=code-1&state=state-other",
    "duplicate_query_fields_keep_the_first": "/callback?code=&code=first&state=&state=state-expected",
    "percent_escaped_values": "/callback?code=a%2Fb%20c&state=state%2Dexpected",
    "blank_query_fields_are_dropped": "/callback?code=&state=",
    "unrelated_path": "/favicon.ico?code=code-1&state=state-expected",
}


def record_case(name: str) -> dict[str, object]:
    server = _CallbackServer(("127.0.0.1", 0), _CallbackHandler)
    server.expected_state = EXPECTED_STATE
    server.callback_result = None
    port = server.server_address[1]
    served = threading.Thread(target=server.handle_request)
    served.start()
    connection = http.client.HTTPConnection("127.0.0.1", port)
    connection.request("GET", TARGETS[name])
    response = connection.getresponse()
    body = response.read().decode()
    connection.close()
    served.join()
    result = server.callback_result
    server.server_close()
    return {
        "name": name,
        "target": TARGETS[name],
        "status": response.status,
        "content_type": response.getheader("Content-Type"),
        "body": body,
        "callback": result,
    }


def main() -> int:
    if version("litellm") != PINNED_VERSION:
        sys.stderr.write(f"expected litellm {PINNED_VERSION}, found {version('litellm')}")
        return 2
    transcript = {
        "source": f"https://github.com/BerriAI/litellm/blob/{PINNED_COMMIT}/litellm/llms/xai/oauth.py#L78-L111",
        "litellm": PINNED_VERSION,
        "expected_state": EXPECTED_STATE,
        "cases": [record_case(name) for name in CASES],
    }
    json.dump(transcript, sys.stdout, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
