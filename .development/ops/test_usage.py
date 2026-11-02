import json
import tempfile
import unittest
from pathlib import Path

from usage import summarize


class UsageTests(unittest.TestCase):
    def test_resumed_goose_session_totals_are_counted_once_but_wall_times_are_added(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            paths = [Path(folder) / f"attempt-{number}-worker.stdout" for number in range(1, 4)]
            for path, cost, tokens in zip(paths[:2], ["5.25", "8.50"], [100, 180], strict=True):
                path.write_text(
                    json.dumps(
                        {
                            "type": "result",
                            "session_id": "goose-session",
                            "total_cost_usd": cost,
                            "modelUsage": {"toy": {"outputTokens": tokens}},
                        }
                    )
                    + "\n"
                )
                path.with_suffix(".finished.json").write_text('{"elapsed_ms":1000,"exit_code":0}')
            paths[2].write_text(
                '{"type":"thread.started","thread_id":"duck"}\n{"type":"turn.completed","usage":{"output_tokens":20}}\n{"interrupted":'
            )
            result = summarize([*paths, paths[0]])
            self.assertEqual(result["invocations"], 3)
            self.assertEqual(result["claude_api_equivalent_usd"], "8.50")
            self.assertEqual(result["claude_output_tokens"], 180)
            self.assertEqual(result["codex_output_tokens"], 20)
            self.assertEqual(result["measured_elapsed_ms"], 2000)


if __name__ == "__main__":
    unittest.main()
