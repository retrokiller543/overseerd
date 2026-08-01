#!/usr/bin/env python3
"""Validate a nextest JUnit report's identity and test count."""

import argparse
from pathlib import Path
import sys
import xml.etree.ElementTree as ElementTree


def local_name(tag):
    return tag.rsplit("}", 1)[-1]


def parse_args():
    parser = argparse.ArgumentParser()

    parser.add_argument("report", type=Path)
    parser.add_argument("expected_report_name")

    return parser.parse_args()


def declared_test_count(root):
    value = root.get("tests")

    if value is not None:
        return int(value)

    return sum(int(suite.get("tests", "0")) for suite in root if local_name(suite.tag) == "testsuite")


def actual_test_count(root):
    return sum(1 for element in root.iter() if local_name(element.tag) == "testcase")


def main():
    args = parse_args()

    if not args.report.is_file():
        print(f"error: JUnit report does not exist: {args.report}", file=sys.stderr)

        return 1

    try:
        root = ElementTree.parse(args.report).getroot()
        declared_count = declared_test_count(root)
        test_count = actual_test_count(root)
    except (ElementTree.ParseError, OSError, ValueError) as error:
        print(f"error: invalid JUnit report {args.report}: {error}", file=sys.stderr)

        return 1

    if local_name(root.tag) != "testsuites":
        print(f"error: expected testsuites root, found {local_name(root.tag)}", file=sys.stderr)

        return 1

    report_name = root.get("name")

    if report_name != args.expected_report_name:
        print(
            f"error: expected report name {args.expected_report_name!r}, found {report_name!r}",
            file=sys.stderr,
        )

        return 1

    if test_count <= 0:
        print(f"error: JUnit report contains no tests: {args.report}", file=sys.stderr)

        return 1

    if declared_count != test_count:
        print(
            f"error: JUnit report declares {declared_count} tests but contains {test_count} testcases",
            file=sys.stderr,
        )

        return 1

    print(f"validated {args.report}: report={report_name!r}, tests={test_count}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
