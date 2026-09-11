#!/usr/bin/env bash
#
# Parse the shell out of every workflow `run:` block and syntax-check it.
#
# A workflow file can be perfectly valid YAML and still carry shell that cannot
# run, because to YAML a `run:` block is just a string. v0.1.1 shipped this:
#
#     echo "... ship through TestFlight.""
#
# — one stray quote, valid YAML, and the release died at the first job with
# `unexpected EOF while looking for matching '`, forty seconds into a run that
# had already been tagged and pushed. This check finds that in well under a
# second, which is the entire argument for it.
#
# `bash -n` parses without executing, so this catches quoting and structure —
# unbalanced quotes, an unclosed if/for/case/heredoc — and nothing about
# whether the commands are correct or the variables exist. That is the deal: a
# cheap check for the failure mode that is pure typo.
#
# Extraction is done with awk rather than a YAML library on purpose: this must
# run anywhere `make test` runs, including a bare debian container, so it may
# not assume ruby, or python with a yaml module, is installed.
set -euo pipefail

cd "$(dirname "$0")/.."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

failed=0
checked=0

for wf in .github/workflows/*.yml .github/workflows/*.yaml; do
  [ -e "$wf" ] || continue

  # Split the file into one script per `run:` block. A block starts at a line
  # matching `run: |` (or `|-`) and continues through every line that is blank
  # or indented deeper than the `run:` key itself.
  rm -f "$tmp"/block.*
  awk -v dir="$tmp" '
    function indent(s,   i) { match(s, /^ */); return RLENGTH }
    /^[ \t]*run:[ \t]*\|-?[ \t]*$/ {
      n++; base = indent($0); inblock = 1; file = dir "/block." n
      printf "" > file
      line[n] = FNR
      next
    }
    inblock {
      if ($0 ~ /^[ \t]*$/) { print "" >> file; next }
      if (indent($0) > base) { print substr($0, base + 3) >> file; next }
      inblock = 0
    }
    END { for (i = 1; i <= n; i++) print i "\t" line[i] > (dir "/index") }
  ' "$wf"

  [ -f "$tmp/index" ] || continue

  while IFS=$'\t' read -r n startline; do
    block="$tmp/block.$n"
    [ -s "$block" ] || continue
    checked=$((checked + 1))
    if ! out="$(bash -n "$block" 2>&1)"; then
      failed=$((failed + 1))
      echo "error: $wf: shell syntax error in the run: block at line $startline" >&2
      # bash names the temp file it was handed; the reader cares about the
      # line within the block, not where we staged it.
      printf '%s\n' "$out" | sed "s|^$block: ||; s/^/       /" >&2
    fi
  done < "$tmp/index"
  rm -f "$tmp/index"
done

if [ "$failed" -gt 0 ]; then
  echo "$failed of $checked run: block(s) will not parse" >&2
  exit 1
fi

echo "workflows ok ($checked run: blocks parse)"
