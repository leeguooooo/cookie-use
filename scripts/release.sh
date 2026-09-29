#!/bin/sh
# Release cookie-use: bump Cargo.toml/Cargo.lock, move CHANGELOG.md's [Unreleased] notes under the new
# version, run the CI checks (fmt, clippy, test), commit "release: vX", push main + the vX tag, wait for
# release-binaries.yml to build and publish the GitHub Release, then sync the plugin marketplace so
# Claude Code plugin installs pick the new version up right away (no token: uses your `gh` login).
#   scripts/release.sh [--dry-run] 0.4.1
set -eu
DRY=
[ "${1:-}" = --dry-run ] && { DRY=1; shift; }
V=${1:?usage: scripts/release.sh [--dry-run] <version>}
REPO=leeguooooo/cookie-use
MARKETPLACE=leeguooooo/plugins
cd "$(dirname "$0")/.."
die() { echo "error: $*" >&2; exit 1; }

[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || die "not on main"
[ -z "$(git status --porcelain)" ] || die "working tree not clean"
git fetch -q origin main
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || die "main is not in sync with origin/main"
[ -z "$(git ls-remote --tags origin "refs/tags/v$V")" ] || die "v$V already exists"
awk '/^## \[Unreleased\]/{f=1;next} /^## \[/{f=0} f&&NF{n++} END{exit !n}' CHANGELOG.md \
  || { [ -n "$DRY" ] || die "CHANGELOG.md has nothing under [Unreleased]"; echo "warn: CHANGELOG.md has nothing under [Unreleased]; a real release stops here" >&2; }
[ -n "$DRY" ] && trap 'git checkout -q -- .' EXIT

sed -i.bak "1,/^version = /s/^version = \".*\"/version = \"$V\"/" Cargo.toml && rm Cargo.toml.bak
cargo update -q -p cookie-use --offline
awk -v h="## [$V] - $(date +%F)" '{print} $0=="## [Unreleased]"{print ""; print h}' CHANGELOG.md > CHANGELOG.md.tmp && mv CHANGELOG.md.tmp CHANGELOG.md
cargo fmt --check
cargo clippy -q -- -D warnings
cargo test -q
if [ -n "$DRY" ]; then git --no-pager diff; echo "dry run: checks done, version bump reverted"; exit 0; fi

git commit -qam "release: v$V"
git tag "v$V"
git push -q origin main "v$V"

# The tag push starts release-binaries.yml, which builds the binaries and CookieUse.dmg and creates the Release.
# Wait for it: the plugin must not update before its binaries exist.
RUN=
for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do
  sleep 5
  RUN=$(gh run list -R "$REPO" -w release-binaries.yml -b "v$V" -e push -L 1 --json databaseId -q '.[0].databaseId')
  [ -n "$RUN" ] && break
done
[ -n "$RUN" ] || die "no release-binaries run for v$V; see https://github.com/$REPO/actions"
echo "waiting for release build $RUN"
gh run watch "$RUN" -R "$REPO" --exit-status >/dev/null || die "release build $RUN failed; marketplace not synced"
echo "released https://github.com/$REPO/releases/tag/v$V"

# The marketplace reads the version from Cargo.toml on main; run its sync now instead of waiting for the hourly cron.
PREV=$(gh run list -R "$MARKETPLACE" -w auto-sync-versions.yml -e workflow_dispatch -L 1 --json databaseId -q '.[0].databaseId')
gh workflow run auto-sync-versions.yml -R "$MARKETPLACE"
RUN=$PREV
for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do
  sleep 5
  RUN=$(gh run list -R "$MARKETPLACE" -w auto-sync-versions.yml -e workflow_dispatch -L 1 --json databaseId -q '.[0].databaseId')
  [ "$RUN" != "$PREV" ] && break
done
gh run watch "$RUN" -R "$MARKETPLACE" --exit-status >/dev/null && echo "marketplace synced" || echo "warn: marketplace sync run $RUN failed; the hourly run will retry"
gh api "repos/$MARKETPLACE/contents/.claude-plugin/marketplace.json" -q .content | base64 -d \
  | python3 -c "import json,sys; print('marketplace cookie-use:', next(p['version'] for p in json.load(sys.stdin)['plugins'] if p['name']=='cookie-use'))"
