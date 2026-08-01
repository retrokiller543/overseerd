#!/usr/bin/env python3
"""Validate nextest cohort and resource-group topology."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys


COHORT_SENTINELS = {
    "critical": {
        ("overseerd-transport", "protocol::codec::tests::rejects_oversized_frame_before_allocating_payload"),
        ("overseerd-rpc", "client::tests::saturated_control_lane_poison_connection_instead_of_allocating"),
        ("overseerd-di", "registry::tests::validate_detects_missing_dependency"),
        ("overseerd-config::substitution", "rendered_output_budget_is_aggregate_across_one_typed_binding"),
    },
    "extended": {
        ("overseerd::config_triggers", "watching_a_source_file_triggers_a_reload"),
        ("overseerd::deprecated_aliases", "daemon_type_alias_builds"),
        (
            "overseerd-example-http::topic_generics",
            "topic_impls_are_generated_for_generic_and_borrowing_sets",
        ),
    },
}

ADVISORY_BINARY_IDS = {
    "overseerd::config_triggers",
    "overseerd::deprecated_aliases",
    "overseerd-example-http::topic_generics",
}

GROUP_SENTINELS = {
    "timing-sensitive": {
        ("overseerd::config_triggers", "watching_a_source_file_triggers_a_reload"),
        ("overseerd-jobs", "scheduler::tests::dynamic_job_runs_then_cancels"),
        (
            "overseerd-transport",
            "protocol::codec::tests::times_out_when_a_frame_stops_making_progress",
        ),
    },
    "filesystem-sensitive": {
        ("overseerd::config_reload", "reload_swaps_only_the_changed_binding"),
    },
    "socket-integration": {
        ("overseerd-example-http::ws", "ws_controller_dispatches_and_injects"),
        (
            "overseerd-example-http::client",
            "generated_client_round_trips_over_reqwest",
        ),
    },
}


def run(command):
    result = subprocess.run(command, check=False, capture_output=True, text=True)

    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()

        raise RuntimeError(f"command failed ({result.returncode}): {' '.join(command)}\n{detail}")

    return result.stdout


def parse_args():
    parser = argparse.ArgumentParser()

    parser.add_argument("--archive-file", type=Path)
    parser.add_argument("--workspace-remap", type=Path)

    return parser.parse_args()


def list_command(args):
    command = ["cargo", "nextest", "list"]

    if args.archive_file is None:
        command.extend(["--workspace", "--all-features", "--locked"])
    else:
        command.extend(["--archive-file", str(args.archive_file)])

        if args.workspace_remap is not None:
            command.extend(["--workspace-remap", str(args.workspace_remap)])

    return command


def list_tests(args, filterset, profile=None, ignore_default_filter=True):
    command = list_command(args)

    if ignore_default_filter:
        command.append("--ignore-default-filter")

    command.extend(["--filterset", filterset, "--message-format", "json"])

    if profile is not None:
        command.extend(["--profile", profile])

    document = json.loads(run(command))
    selected = set()

    for binary_id, suite in document["rust-suites"].items():
        for test_name, testcase in suite["testcases"].items():
            status = testcase["filter-match"]["status"]

            if not testcase["ignored"] and status == "matches":
                selected.add((binary_id, test_name))

    return selected


def list_binary_ids(args):
    command = list_command(args)

    command.extend([
        "--list-type",
        "binaries-only",
        "--message-format",
        "json",
    ])
    document = json.loads(run(command))

    return set(document["rust-binaries"])


def require_subset(errors, expected, actual, description):
    missing = expected - actual

    if missing:
        rendered = ", ".join(f"{binary_id}::{test_name}" for binary_id, test_name in sorted(missing))
        errors.append(f"{description} missing sentinels: {rendered}")


def summary_table(selections):
    lines = [
        "## Nextest topology",
        "",
        "| Selection | Tests | Binaries |",
        "|---|---:|---:|",
    ]

    for name, tests in selections:
        binary_count = len({binary_id for binary_id, _ in tests})

        lines.append(f"| {name} | {len(tests)} | {binary_count} |")

    return "\n".join(lines) + "\n"


def append_summary(summary):
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")

    if summary_path:
        with Path(summary_path).open("a", encoding="utf-8") as output:
            output.write(summary)


def platform_group_sentinels():
    sentinels = set()

    if sys.platform == "win32":
        sentinels.add(
            (
                "overseerd-dirs",
                "tests::private_directories_receive_and_retain_a_private_windows_acl",
            )
        )
    else:
        sentinels.add(
            (
                "overseerd-transport",
                "transports::unix::tests::socket_and_parent_are_private",
            )
        )

    return sentinels


def main():
    args = parse_args()
    errors = []
    critical_filter = run(["just", "--evaluate", "critical_filter"]).strip()
    critical_binary_ids = set(re.findall(r"binary_id\(=([^\)]+)\)", critical_filter))
    full = list_tests(args, "all()")
    critical = list_tests(args, critical_filter)
    extended = list_tests(args, f"all() - ({critical_filter})")
    effective_critical = list_tests(
        args,
        critical_filter,
        profile="ci-critical",
        ignore_default_filter=False,
    )
    effective_extended = list_tests(
        args,
        f"all() - ({critical_filter})",
        profile="ci-extended",
        ignore_default_filter=False,
    )
    binary_ids = list_binary_ids(args)
    groups = {
        name: list_tests(args, f"group(={name})", profile="ci-critical")
        for name in GROUP_SENTINELS
    }

    if not full:
        errors.append("full selection is empty")

    if not critical:
        errors.append("critical selection is empty")

    if not extended:
        errors.append("extended selection is empty")

    overlap = critical & extended

    if overlap:
        errors.append(f"critical and extended overlap on {len(overlap)} tests")

    if critical | extended != full:
        missing = full - (critical | extended)
        unexpected = (critical | extended) - full
        errors.append(
            "critical and extended do not partition full selection "
            f"({len(missing)} missing, {len(unexpected)} unexpected)"
        )

    if effective_critical != critical:
        errors.append("ci-critical effective selection differs from the canonical critical cohort")

    if effective_extended != extended:
        errors.append("ci-extended effective selection differs from the canonical extended cohort")

    missing_critical_binaries = critical_binary_ids - binary_ids

    if missing_critical_binaries:
        errors.append(
            "critical filter references missing binaries: "
            + ", ".join(sorted(missing_critical_binaries))
        )

    if "overseerd::type_registration" not in binary_ids:
        errors.append("critical empty-target sentinel is missing: overseerd::type_registration")

    require_subset(errors, COHORT_SENTINELS["critical"], critical, "critical cohort")
    require_subset(errors, COHORT_SENTINELS["extended"], extended, "extended cohort")

    missing_advisory_binaries = ADVISORY_BINARY_IDS - binary_ids

    if missing_advisory_binaries:
        errors.append(
            "advisory binaries are missing: " + ", ".join(sorted(missing_advisory_binaries))
        )

    for binary_id in sorted(ADVISORY_BINARY_IDS):
        advisory_tests = {test for test in full if test[0] == binary_id}

        if not advisory_tests:
            errors.append(f"advisory binary has no selected tests: {binary_id}")
        elif not advisory_tests <= extended:
            errors.append(f"advisory binary is not entirely extended: {binary_id}")

    for name, expected in GROUP_SENTINELS.items():
        selected = groups[name]

        if not selected:
            errors.append(f"resource group is empty: {name}")

        require_subset(errors, expected, selected, f"resource group {name}")

    require_subset(
        errors,
        platform_group_sentinels(),
        groups["filesystem-sensitive"],
        "resource group filesystem-sensitive",
    )

    selections = [
        ("Full", full),
        ("Critical", critical),
        ("Extended", extended),
        *((name, tests) for name, tests in groups.items()),
    ]
    summary = summary_table(selections)

    print(summary, end="")
    append_summary(summary)

    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)

        return 1

    return 0


if __name__ == "__main__":
    try:
        exit_code = main()
    except (json.JSONDecodeError, OSError, RuntimeError) as error:
        print(f"error: {error}", file=sys.stderr)
        exit_code = 1

    sys.exit(exit_code)
