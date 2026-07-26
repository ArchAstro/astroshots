#!/usr/bin/env bash
# Public contract proof for agent-readable human review feedback.
#
# This validates the documented JSON shape and content-hash invalidation. It
# does not simulate the Astroshots UI or claim that automation reviewed an
# image; the approved decision below is a fixture representing human-authored
# state.
set -euo pipefail

command -v jq >/dev/null 2>&1 || {
  echo "test-review-contract: jq is required" >&2
  exit 1
}

TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/astroshots-review-contract.XXXXXX")"
cleanup() {
  rm -rf "$TEST_ROOT"
}
trap cleanup EXIT

FEATURE_DIR="$TEST_ROOT/.astroshot/review-contract"
IMAGE="$FEATURE_DIR/0001-dialog.png"
MANIFEST="$FEATURE_DIR/manifest.json"
REVIEW="$FEATURE_DIR/review.json"
mkdir -p "$FEATURE_DIR"

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

read_review() {
  local file="$1"
  local current_sha
  local manifest_run_id
  current_sha="$(hash_file "$FEATURE_DIR/$file")"
  manifest_run_id="$(jq -r '.run_id // empty' "$MANIFEST" 2>/dev/null || true)"

  if [[ ! -f "$REVIEW" ]]; then
    jq -nc '{state: "pending", comments: []}'
    return
  fi

  jq -c --arg file "$file" --arg sha "$current_sha" \
    --arg manifest_run_id "$manifest_run_id" '
    if $manifest_run_id != "" and (.run_id // "") != $manifest_run_id then
      {state: "pending", comments: []}
    else
      (.reviews[$file] // null) as $review
      | {
          state: (
            if $review == null then "pending"
            elif $review.decision == null then "pending"
            elif $review.image_sha256 != $sha then "stale"
            else $review.decision
            end
          ),
          comments: ($review.comments // [])
        }
    end
  ' "$REVIEW"
}

# The manifest can pass while human review independently requests changes.
printf 'review contract image version one\n' >"$IMAGE"
IMAGE_SHA="$(hash_file "$IMAGE")"
jq -n '{
  version: 1,
  feature: "review-contract",
  run_id: "review-contract-run",
  status: "pass",
  shots: [{id: "0001", file: "0001-dialog.png", slug: "dialog"}]
}' >"$MANIFEST"

MISSING="$(read_review "0001-dialog.png")"
[[ "$(jq -r '.state' <<<"$MISSING")" == "pending" ]] ||
  { echo "expected a missing review sidecar to be pending" >&2; exit 1; }
[[ "$(jq -r '.comments | length' <<<"$MISSING")" == "0" ]] ||
  { echo "expected a missing review sidecar to expose no comments" >&2; exit 1; }

# Fixture representing a decision written by a human through Astroshots.
jq -n --arg sha "$IMAGE_SHA" '{
  version: 1,
  run_id: "review-contract-run",
  updated_at: "2026-07-26T17:42:00Z",
  reviews: {
    "0001-dialog.png": {
      decision: "approved",
      reviewed_at: "2026-07-26T17:42:00Z",
      image_sha256: $sha,
      comments: [{
        id: "comment-1",
        body: "Keep the focused crop.",
        created_at: "2026-07-26T17:41:32Z"
      }]
    },
    "0002-comment-only.png": {
      comments: [{
        id: "comment-only-1",
        body: "Check the empty state copy.",
        created_at: "2026-07-26T17:41:40Z"
      }]
    }
  }
}' >"$REVIEW"

jq -e '
  .version == 1
  and ((.run_id == null) or (.run_id | type == "string" and length > 0))
  and ((.updated_at == null) or (.updated_at | type == "string" and length > 0))
  and (.reviews | type == "object")
  and all(
    .reviews | to_entries[];
    (.key | test("^[^/]+$"))
    and (
      (.value.decision == null)
      or (.value.decision == "approved")
      or (.value.decision == "changes_requested")
    )
    and (
      if .value.decision == null then
        (.value.reviewed_at == null)
        and (.value.image_sha256 == null)
      else
        (.value.reviewed_at | type == "string" and length > 0)
        and (.value.image_sha256 | test("^[0-9a-f]{64}$"))
      end
    )
    and (.value.comments | type == "array")
    and all(
      .value.comments[];
      (.id | type == "string" and length > 0)
      and (.body | type == "string" and length > 0)
      and (.created_at | type == "string" and length > 0)
    )
  )
' "$REVIEW" >/dev/null

printf 'comment-only image\n' >"$FEATURE_DIR/0002-comment-only.png"
COMMENT_ONLY="$(read_review "0002-comment-only.png")"
[[ "$(jq -r '.state' <<<"$COMMENT_ONLY")" == "pending" ]] ||
  { echo "expected comment-only feedback without a hash to remain pending" >&2; exit 1; }
[[ "$(jq -r '.comments[0].body' <<<"$COMMENT_ONLY")" == "Check the empty state copy." ]] ||
  { echo "expected agent reader to expose comment-only feedback" >&2; exit 1; }

CURRENT="$(read_review "0001-dialog.png")"
[[ "$(jq -r '.state' <<<"$CURRENT")" == "approved" ]] ||
  { echo "expected matching human decision to be approved" >&2; exit 1; }
[[ "$(jq -r '.comments[0].body' <<<"$CURRENT")" == "Keep the focused crop." ]] ||
  { echo "expected agent reader to expose the human comment" >&2; exit 1; }

# The same image bytes in a different capture run are a new review subject.
# Old run-scoped approval and comments must both be suppressed.
jq '.run_id = "review-contract-run-2"' "$MANIFEST" >"$MANIFEST.next"
mv "$MANIFEST.next" "$MANIFEST"
[[ "$(hash_file "$IMAGE")" == "$IMAGE_SHA" ]] ||
  { echo "new-run proof must keep identical image bytes" >&2; exit 1; }
NEW_RUN="$(read_review "0001-dialog.png")"
[[ "$(jq -r '.state' <<<"$NEW_RUN")" == "pending" ]] ||
  { echo "expected an identical image in a new run to be pending" >&2; exit 1; }
[[ "$(jq -r '.comments | length' <<<"$NEW_RUN")" == "0" ]] ||
  { echo "expected prior-run comments to be suppressed" >&2; exit 1; }

# Restore the matching run before proving content-hash invalidation.
jq '.run_id = "review-contract-run"' "$MANIFEST" >"$MANIFEST.next"
mv "$MANIFEST.next" "$MANIFEST"

# Replacing bytes at the same filename invalidates the decision, not comments.
printf 'review contract image version two\n' >"$IMAGE"
STALE="$(read_review "0001-dialog.png")"
[[ "$(jq -r '.state' <<<"$STALE")" == "stale" ]] ||
  { echo "expected changed image bytes to invalidate approval" >&2; exit 1; }
[[ "$(jq -r '.comments[0].body' <<<"$STALE")" == "Keep the focused crop." ]] ||
  { echo "expected stale review comments to remain agent-readable" >&2; exit 1; }

# A later human-authored decision for the new hash remains independent of the
# successful execution status in manifest.json.
NEW_SHA="$(hash_file "$IMAGE")"
jq --arg sha "$NEW_SHA" '
  .updated_at = "2026-07-26T17:44:00Z"
  | .reviews["0001-dialog.png"].decision = "changes_requested"
  | .reviews["0001-dialog.png"].reviewed_at = "2026-07-26T17:44:00Z"
  | .reviews["0001-dialog.png"].image_sha256 = $sha
  | .reviews["0001-dialog.png"].comments += [{
      id: "comment-2",
      body: "Restore the missing primary action.",
      created_at: "2026-07-26T17:43:50Z"
    }]
' "$REVIEW" >"$REVIEW.next"
mv "$REVIEW.next" "$REVIEW"

REQUESTED="$(read_review "0001-dialog.png")"
[[ "$(jq -r '.state' <<<"$REQUESTED")" == "changes_requested" ]] ||
  { echo "expected current human decision to request changes" >&2; exit 1; }
[[ "$(jq -r '.comments | length' <<<"$REQUESTED")" == "2" ]] ||
  { echo "expected ordered feedback history to remain readable" >&2; exit 1; }
[[ "$(jq -r '.status' "$MANIFEST")" == "pass" ]] ||
  { echo "execution manifest must remain independent of review decision" >&2; exit 1; }

echo "PASS: review contract missing state, run scope, feedback, and hash invalidation"
