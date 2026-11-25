"""Run this crate's test files with stable cargo test and write JUnit XML.

Stable libtest has no JUnit output. Preserve cargo test --locked and its
default handling of #[ignore], translating its explicit per-test statuses.
Unit tests live in inline `mod tests` blocks; integration tests are tests/*.rs.
"""

import re
import subprocess
import sys
import time
import xml.etree.ElementTree as ET
from pathlib import Path


def discover(directory):
    files = subprocess.check_output(
        ["git", "ls-files", "-z", "--", directory], text=True
    ).split("\0")
    return sorted(
        filename
        for filename in files
        if filename.endswith(".rs")
        and re.search(r"^\s*#\[test\]", Path(filename).read_text(), re.MULTILINE)
    )


def command_for(filename):
    path = Path(filename)
    command = ["cargo", "test", "--locked"]
    if path.parts[0] == "src":
        module = list(path.with_suffix("").parts[1:])
        if module[-1] in ("main", "mod"):
            module.pop()
        test_filter = "::".join([*module, "tests", ""])
        command += ["--bin", "opencl_vanity_gpg", "--", test_filter]
    elif path.parent == Path("tests"):
        command += ["--test", path.stem, "--"]
    else:
        raise ValueError(f"Unsupported test file: {filename}")
    return command + ["--format", "pretty", "--color", "never"]


def run(report, filenames):
    known_files = set(discover("src") + discover("tests"))
    if not filenames or not set(filenames) <= known_files:
        raise ValueError("Expected one or more discovered Rust test files")

    results = ET.Element("testsuites")
    failed = False
    for filename in filenames:
        command = command_for(filename)
        print("+ " + " ".join(command), flush=True)
        started = time.monotonic()
        result = subprocess.run(
            command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True
        )
        print(result.stdout, end="", flush=True)
        suite = ET.SubElement(results, "testsuite", name=filename)
        statuses = re.findall(
            r"^test (\S+) \.\.\. (ok|FAILED|ignored)(?:, (.*))?$",
            result.stdout,
            re.MULTILINE,
        )
        for name, status, reason in statuses:
            case = ET.SubElement(
                suite, "testcase", name=name, classname=filename, file=filename
            )
            if status == "ignored":
                ET.SubElement(case, "skipped", message=reason or "#[ignore]")
            elif status == "FAILED":
                ET.SubElement(case, "failure", message="Rust test failed").text = (
                    result.stdout
                )

        # Fail closed on compilation errors, crashes, or unrecognized output.
        summary = re.search(
            r"^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;",
            result.stdout,
            re.MULTILINE,
        )
        reported_count = sum(map(int, summary.groups())) if summary else -1
        if not statuses or reported_count != len(statuses) or (
            result.returncode and not any(s == "FAILED" for _, s, _ in statuses)
        ):
            case = ET.SubElement(
                suite, "testcase", name="cargo test", classname=filename, file=filename
            )
            ET.SubElement(case, "error", message="Cargo did not report all tests").text = (
                result.stdout
            )

        failures = len(suite.findall("testcase/failure"))
        errors = len(suite.findall("testcase/error"))
        suite.set("tests", str(len(suite.findall("testcase"))))
        suite.set("failures", str(failures))
        suite.set("errors", str(errors))
        suite.set("skipped", str(len(suite.findall("testcase/skipped"))))
        suite.set("time", f"{time.monotonic() - started:.6f}")
        failed |= bool(result.returncode or failures or errors)

    report_path = Path(report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(results).write(report_path, encoding="utf-8", xml_declaration=True)
    return int(failed)


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "discover":
        print("\n".join(discover(sys.argv[2])))
    elif len(sys.argv) >= 4 and sys.argv[1] == "run":
        sys.exit(run(sys.argv[2], sys.argv[3:]))
    else:
        sys.exit("Usage: cargo-tests.py discover <src|tests> | run <junit.xml> <files...>")
