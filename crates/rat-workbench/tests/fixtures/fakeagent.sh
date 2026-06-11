#!/usr/bin/env bash
# fakeagent.sh — deterministic fake agent for integration tests.
#
# Usage: bash fakeagent.sh
#   Run from inside a git worktree. The working directory is the worktree.
#
# If FAKEAGENT_SCRIPT is set, it must contain newline-separated directives:
#   write:<relative-path>:<content>   — write the given content to the file
#   commit:<message>                   — git add -A && git commit -m <message>
#   exit:<code>                        — exit with the given integer code
#
# Default behaviour (FAKEAGENT_SCRIPT not set or empty):
#   1. Write AGENT_NOTE.md with a fixed message.
#   2. git add AGENT_NOTE.md
#   3. git commit -m "fakeagent: scripted change"
#   4. exit 0

set -euo pipefail

if [ -z "${FAKEAGENT_SCRIPT:-}" ]; then
    echo "fakeagent: no FAKEAGENT_SCRIPT set, running default behaviour"
    printf 'Written by fakeagent.\n' > AGENT_NOTE.md
    git add AGENT_NOTE.md
    git commit -m "fakeagent: scripted change"
    exit 0
fi

echo "fakeagent: executing script"

while IFS= read -r line || [ -n "$line" ]; do
    # Skip blank lines and comments
    case "$line" in
        ''|\#*) continue ;;
    esac

    directive="${line%%:*}"
    rest="${line#*:}"

    case "$directive" in
        write)
            # rest = "<path>:<content>"
            filepath="${rest%%:*}"
            content="${rest#*:}"
            # Ensure parent directory exists
            mkdir -p "$(dirname "$filepath")"
            printf '%s\n' "$content" > "$filepath"
            echo "fakeagent: wrote $filepath"
            ;;
        commit)
            msg="$rest"
            git add -A
            git commit -m "$msg"
            echo "fakeagent: committed '$msg'"
            ;;
        exit)
            code="$rest"
            echo "fakeagent: exiting with code $code"
            exit "$code"
            ;;
        *)
            echo "fakeagent: unknown directive '$directive', ignoring" >&2
            ;;
    esac
done <<< "$FAKEAGENT_SCRIPT"

exit 0
