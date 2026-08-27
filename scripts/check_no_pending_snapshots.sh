#!/usr/bin/env bash
# Refuse a commit that leaves insta snapshots unreviewed.
#
# The output format is the product: an unreviewed .snap.new means someone changed
# what every caller sees and did not look at the diff.
set -euo pipefail

pending=$(find . -name '*.snap.new' -not -path './target/*' -print)

if [ -n "$pending" ]; then
  echo "unreviewed snapshot changes:" >&2
  echo "$pending" >&2
  echo >&2
  echo "Run 'just snapshots' (cargo insta review) and read the diff." >&2
  exit 1
fi
