#!/usr/bin/env python3
"""Lists the failing checks of an `esp-idf-sys` build with `ESP_IDF_LIBC_MISMATCHES=err`.

Usage: libc_mismatches.py <exit code of cargo> <messages of cargo> <title>

The messages are the output of `cargo build --message-format=json...`. The list goes to the log
and, under GitHub Actions, to the job summary and an annotation. Fails if the build failed.
"""

import json
import os
import sys

# The checks of `esp-idf-sys/src/checks/libc.rs`, and the ones it includes from the build script
CHECK_FILES = ("src/checks/libc.rs", "/libc_checks.rs")


def span_files(span):
    """The file of a span, and the files of the macro expansions it comes from"""
    while span:
        yield span["file_name"]
        span = (span.get("expansion") or {}).get("span")


def is_check(message):
    return any(
        file.endswith(CHECK_FILES)
        for span in message["spans"]
        if span["is_primary"]
        for file in span_files(span)
    )


def main():
    status, messages_file, title = int(sys.argv[1]), sys.argv[2], sys.argv[3]

    checks, others = [], []
    with open(messages_file, encoding="utf-8") as file:
        for line in file:
            try:
                msg = json.loads(line)
            except ValueError:
                continue

            if msg.get("reason") != "compiler-message" or msg["message"]["level"] != "error":
                continue

            message = msg["message"]
            if is_check(message):
                text = message["message"].removeprefix("evaluation panicked: ")
                if text not in checks:
                    checks.append(text)
            else:
                others.append(message.get("rendered") or message["message"])

    lines = [f"### {title}: {len(checks)} known `libc` mismatches", ""]
    lines += [f"- {text}" for text in checks]
    report = "\n".join(lines) + "\n"

    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as file:
            file.write(report + "\n")

    # The page of the run shows the annotation, rather than "Process completed with exit code 1"
    if checks and os.environ.get("GITHUB_ACTIONS"):
        print(f"::error::{len(checks)} known libc mismatches, listed in the job summary")

    if status == 0:
        return 0

    if others:
        print("The build failed for other reasons as well:\n")
        print("\n".join(others))
    elif not checks:
        print("The build failed before compiling the checks against `libc` (see the log above)")

    return 1


if __name__ == "__main__":
    sys.exit(main())
