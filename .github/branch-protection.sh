#!/usr/bin/env bash
#
# One-time setup of `master` branch protection for this repo.
#
# Run this AFTER the PR that adds .github/workflows/ci.yaml has merged: GitHub
# only lets you require a status-check context that it has already seen report
# at least once, so the check names below must exist before they can be made
# required.
#
# Re-running is safe — it overwrites the protection with the same settings.
#
# Requires: gh (authenticated with admin on the repo).
set -euo pipefail

REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
BRANCH="master"

# Hard blockers — a PR cannot merge into master until these pass.
# Matrix jobs report one context per leg: "Build (<crate>)".
# Clippy and Examples are intentionally omitted, so they stay advisory.
REQUIRED_CONTEXTS=(
  "Formatting"
  "Test suite"
  "Build (overseerd)"
  "Build (overseerd-config)"
  "Build (overseerd-core)"
  "Build (overseerd-macros)"
  "Build (overseerd-transport)"
  "Build (overseerd-analyze)"
)

contexts_json="$(printf '%s\n' "${REQUIRED_CONTEXTS[@]}" \
  | jq -R . | jq -s .)"

# enforce_admins=true  -> direct pushes to master are blocked for everyone,
#                         admins included; all changes go through a PR.
# required_approving_review_count=0 -> a solo maintainer can still self-merge
#                         their own PR once the required checks are green.
# strict=false -> do not force a branch to be rebased onto latest master before
#                 merge (flip to true if you want that; adds friction).
jq -n \
  --argjson contexts "$contexts_json" \
  '{
    required_status_checks: { strict: false, contexts: $contexts },
    enforce_admins: true,
    required_pull_request_reviews: { required_approving_review_count: 0 },
    restrictions: null,
    required_linear_history: false,
    allow_force_pushes: false,
    allow_deletions: false
  }' \
  | gh api -X PUT "repos/${REPO}/branches/${BRANCH}/protection" \
      -H "Accept: application/vnd.github+json" --input -

echo "Branch protection applied to ${REPO}@${BRANCH}."