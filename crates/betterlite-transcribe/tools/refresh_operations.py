from pathlib import Path

import botocore.session

# /// script
# requires-python = ">=3.11,<3.14"
# dependencies = ["botocore==1.43.118", "botocore-stubs==1.43.118"]
# ///

ROOT = Path(__file__).resolve().parents[1]


def main() -> None:
    operations: list[str] = botocore.session.get_session().get_service_model("transcribe").operation_names
    _ = (ROOT / "data" / "operations.txt").write_text("".join(f"{name}\n" for name in sorted(operations)))


if __name__ == "__main__":
    main()
