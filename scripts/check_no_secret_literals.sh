#!/usr/bin/env bash
# Refuse obvious credential literals in source.
#
# gitleaks catches high-entropy strings; this catches the shape it tolerates —
# a token assigned to a plausibly named binding. Tests are excluded by the hook
# configuration, since fixtures legitimately contain fake tokens.
set -euo pipefail

pattern='(?i)\b(token|secret|password|api_?key|oauth)\b[[:space:]]*[:=][[:space:]]*"[^"]{16,}"'
status=0

for file in "$@"; do
  if matches=$(grep -nEi "$pattern" "$file" 2>/dev/null); then
    echo "hardcoded credential literal in $file:" >&2
    echo "$matches" >&2
    status=1
  fi
done

if [ "$status" -ne 0 ]; then
  echo >&2
  echo "Secrets belong in the OS keychain (ytcli auth login) or in .env, never in source." >&2
fi

exit "$status"
