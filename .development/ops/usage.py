import argparse
import json
from decimal import Decimal
from pathlib import Path
from typing import cast


def events(path: Path) -> list[dict[str, object]]:
    found: list[dict[str, object]] = []
    with path.open() as stream:
        for line in stream:
            try:
                value = cast(object, json.loads(line, parse_float=Decimal))
            except json.JSONDecodeError:
                continue  # An interrupted final line is not a completed usage record.
            if isinstance(value, dict):
                found.append(cast(dict[str, object], value))
    return found


def summarize(paths: list[Path]) -> dict[str, object]:
    costs: dict[str, Decimal] = {}
    claude_output: dict[str, int] = {}
    codex_output = 0
    elapsed_ms = 0
    incomplete = 0
    unique = sorted({path.resolve() for path in paths})
    for path in unique:
        completed = False
        for event in events(path):
            if event.get("type") == "result":
                completed = True
                session = str(event.get("session_id") or path)
                cost = Decimal(str(event.get("total_cost_usd", 0)))
                costs[session] = max(costs.get(session, Decimal(0)), cost)
                model_usage = event.get("modelUsage")
                if isinstance(model_usage, dict):
                    models = cast(dict[str, object], model_usage)
                    tokens = sum(
                        int(str(cast(dict[str, object], usage).get("outputTokens", 0)))
                        for usage in models.values()
                        if isinstance(usage, dict)
                    )
                    claude_output[session] = max(claude_output.get(session, 0), tokens)
            elif event.get("type") == "turn.completed":
                completed = True
                usage = event.get("usage")
                if isinstance(usage, dict):
                    codex_output += int(str(cast(dict[str, object], usage).get("output_tokens", 0)))
        finished = path.with_suffix(".finished.json")
        if finished.exists():
            elapsed_ms += int(str(events(finished)[0]["elapsed_ms"]))
        if not completed:
            incomplete += 1
    return {
        "invocations": len(unique),
        "claude_sessions": len(costs),
        "claude_api_equivalent_usd": str(sum(costs.values(), Decimal(0))),
        "claude_output_tokens": sum(claude_output.values()),
        "codex_output_tokens": codex_output,
        "measured_elapsed_ms": elapsed_ms,
        "incomplete_usage_records": incomplete,
    }


class Arguments(argparse.Namespace):
    journal: Path


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="Summarize journal usage without counting resumed Claude session totals twice."
    )
    parser.add_argument("journal", type=Path)
    args = parser.parse_args(namespace=Arguments())
    print(json.dumps(summarize(list(args.journal.glob("**/attempt-*.stdout"))), indent=2))
