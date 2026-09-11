#!/usr/bin/env bash
#
# Cut a release: pick a version, stamp it everywhere, tag it, push it, and hand
# the rest to CI.
#
# Pushing the tag is the whole point. `.github/workflows/release.yml` triggers
# on `v*`, and a tag-push run sets publish=true, so everything after this
# script — building every platform, signing and notarizing the .dmg, attaching
# the artifacts, writing manifest.json, publishing the release — happens on
# GitHub. Nothing here builds anything.
#
#   make version            prompt, defaulting to the next patch
#   make version V=0.2.0    take it from the command line instead
#
# VERSIONED_FILES is passed in by the Makefile, which owns that list.
set -euo pipefail

cd "$(dirname "$0")/.."

# --- pick the version -------------------------------------------------------

cur="$(tr -d '\r\n' < VERSION)"
major="${cur%%.*}"; rest="${cur#*.}"; minor="${rest%%.*}"; patch="${rest##*.}"
def="$major.$minor.$((patch + 1))"

printf 'Current version: %s\n' "$cur"

v="${V:-}"
if [ -z "$v" ]; then
  if [ ! -t 0 ]; then
    echo "error: no tty to prompt on — pass the version instead: make version V=x.y.z" >&2
    exit 1
  fi
  printf 'New version [%s]: ' "$def"
  read -r v
  v="${v:-$def}"
fi

[[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.]+)?$ ]] || {
  echo "error: not an x.y.z version: $v" >&2; exit 1; }

tag="v$v"

# --- refuse to cut a release from a tree that isn't the one being released ---
#
# Each of these is a way to tag something other than what you think you are
# tagging, and all of them are cheaper to catch here than to explain from a
# published release.

branch="$(git symbolic-ref --quiet --short HEAD || true)"
[ -n "$branch" ] || { echo "error: HEAD is detached; check out a branch first" >&2; exit 1; }

# Uncommitted work is not in the commit that gets tagged. Releasing anyway
# means the release silently lacks it.
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "error: working tree has uncommitted changes; they would not be in $tag" >&2
  git status --short --untracked-files=no >&2
  exit 1
fi

echo ">> fetching origin…"
git fetch --quiet origin --tags --prune

# A tag on the remote that is not here yet fails at the push, after the commit
# has already landed — so ask about both.
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "error: tag $tag already exists locally" >&2; exit 1
fi
if [ -n "$(git ls-remote --tags origin "refs/tags/$tag")" ]; then
  echo "error: tag $tag already exists on origin" >&2
  echo "       (delete it with: git push origin :refs/tags/$tag)" >&2
  exit 1
fi

# Behind the remote means releasing code that is not what origin/main holds.
upstream="origin/$branch"
if git rev-parse -q --verify "$upstream" >/dev/null; then
  behind="$(git rev-list --count "HEAD..$upstream")"
  if [ "$behind" -gt 0 ]; then
    echo "error: $branch is $behind commit(s) behind $upstream; rebase before releasing" >&2
    exit 1
  fi
fi

# --- stamp, commit, tag, push -----------------------------------------------

./scripts/bump-version.sh "$v"

# shellcheck disable=SC2086
git add ${VERSIONED_FILES:-VERSION}

# Nothing staged means the files already said $v — the normal case for a first
# release, or after a manual `make bump-version`. That is not an error, and it
# used to be: `git commit` exits non-zero on an empty commit, which killed the
# && chain this script replaced *before it tagged or pushed anything*, so
# `make version` could not cut a version the tree already carried.
if git diff --cached --quiet; then
  echo ">> version files already say $v; tagging the commit that is there"
else
  git commit -m "release $tag"
fi

git tag "$tag"

echo ">> pushing ${branch}…"
git push origin "$branch"
echo ">> pushing ${tag}…"
git push origin "$tag"

# --- hand off ---------------------------------------------------------------

origin_url="$(git remote get-url origin)"
origin_url="${origin_url%.git}"
repo=""
case "$origin_url" in
  *github.com[:/]*) repo="${origin_url#*github.com}"; repo="${repo#[:/]}" ;;
esac

printf '\nPushed %s.\n\n' "$tag"
echo "CI takes it from here: build every platform, sign and notarize the .dmg,"
echo "attach the artifacts to a draft release, write manifest.json, and publish."

# Only when origin actually is GitHub — a local or mirror remote would
# otherwise be printed as a github.com URL that does not exist.
if [ -n "$repo" ]; then
  echo
  echo "  https://github.com/$repo/actions/workflows/release.yml"
  echo "  https://github.com/$repo/releases/tag/$tag"
fi

cat <<'EOF'

The release stays a draft until every platform job has succeeded, so an empty
releases page means it is still building — or that something failed and
nothing was published, which is the same page either way. Watch the run.
EOF
