from typing import Any, cast

SQLSTATE_LENGTH = 5
TRANSPORT_WORDS = (
    "can't reach database server",
    "cannot reach database server",
    "can't connect",
    "cannot connect",
    "connection error",
    "connection closed",
    "timed out",
    "timeout",
    "connection refused",
    "network is unreachable",
    "no route to host",
    "broken pipe",
)
DATA_TYPES = {
    "data",
    "unique_violation",
    "foreign_key_violation",
    "missing_required_value",
    "raw_query",
    "table_not_found",
    "record_not_found",
}
PRISMA_TYPES = DATA_TYPES | {"prisma"}


def run(input_value: dict[str, Any]) -> dict[str, Any]:
    error = cast(dict[str, object], input_value["error"])
    kind = cast(str, error["error_type"])
    message = cast(str, error.get("message", ""))
    lower = message.lower()
    connection = kind in {"connection", "deadline", "engine_connection", "no_db_connection"}
    data = kind == "data"
    infrastructure = kind in {
        "connection",
        "deadline",
        "engine_connection",
        "no_db_connection",
        "prisma",
    }
    transport_types = {
        "connection",
        "deadline",
        "engine_connection",
        "no_db_connection",
        "client_not_connected",
        "http_client_closed",
    }
    transport = kind in transport_types or (
        kind == "prisma" and any(word in lower for word in TRANSPORT_WORDS)
    )
    deadlock_words = ("deadlock detected", "40p01", "write conflict or a deadlock")
    deadlock = kind in PRISMA_TYPES and (
        error.get("code") == "P2034" or any(word in lower for word in deadlock_words)
    )
    sqlstate = error.get("meta_code") if data else None
    if data and sqlstate is None:
        marker = 'PostgresError { code: "'
        start = message.find(marker)
        if start >= 0:
            candidate = message[start + len(marker) : start + len(marker) + SQLSTATE_LENGTH]
            if len(candidate) == SQLSTATE_LENGTH and all(
                char.isupper() or char.isdigit() for char in candidate
            ):
                sqlstate = candidate
    engine_internal = kind not in PRISMA_TYPES and any(
        module.startswith("prisma.engine")
        for module in cast(list[str], error.get("traceback_modules", []))
    )
    return {
        "connection": connection,
        "infrastructure": infrastructure and kind not in DATA_TYPES,
        "data": data,
        "transport": transport,
        "prisma": kind in PRISMA_TYPES,
        "deadlock": deadlock,
        "sqlstate": sqlstate,
        "read_only": kind in PRISMA_TYPES
        and ('"25006"' in lower or "read-only transaction" in lower),
        "engine_internal": engine_internal,
        "permanent": infrastructure and kind not in DATA_TYPES and not connection and not transport,
    }
