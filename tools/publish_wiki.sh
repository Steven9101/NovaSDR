#!/usr/bin/env bash
set -euo pipefail

REPO_SLUG="${1:-Steven9101/NovaSDR}"
WIKI_URL="https://github.com/${REPO_SLUG}.wiki.git"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

cd "$(git rev-parse --show-toplevel)"

if ! git clone "$WIKI_URL" "$tmpdir/wiki"; then
  cat >&2 <<EOF
Failed to clone the wiki repository: $WIKI_URL

This usually means the GitHub wiki is disabled for the repo or you don't have access.
Enable it in: Settings -> Features -> Wikis, then rerun:
  tools/publish_wiki.sh ${REPO_SLUG}
EOF
  exit 2
fi

rsync -a --delete --exclude='.git/' docs/wiki/ "$tmpdir/wiki/"
if [[ -d "docs/assets" ]]; then
  rsync -a --delete --exclude='.git/' docs/assets/ "$tmpdir/wiki/assets/"
fi

for f in docs/*.md; do
  base="$(basename "$f")"
  name="${base%.md}"

  if [[ "$name" == "index" ]]; then
    out="$tmpdir/wiki/Home.md"
  else
    out="$tmpdir/wiki/${name}.md"
  fi

  sed -E \
    -e 's/\]\(index\.md\)/](Home)/g' \
    -e 's/\]\(([^)]+)\.md\)/](\1)/g' \
    "$f" >"$out"
done

pushd "$tmpdir/wiki" >/dev/null
if [[ -n "$(git status --porcelain=v1)" ]]; then
  git add -A
  git commit -m "Sync wiki from repo docs"
  git push origin HEAD
else
  echo "Wiki already up to date."
fi
popd >/dev/null
