#!/usr/bin/env bash
# Seed the project's GitHub Wiki from the in-repo /docs folder.
#
# Run once after you've enabled Wikis in the repo settings *and* created the
# very first wiki page through the GitHub UI (without that page, the wiki
# repo is uninitialized and `git clone` fails).
#
# Usage:
#   ./scripts/seed-wiki.sh                 # uses origin remote
#   REPO=owner/name ./scripts/seed-wiki.sh # overrides

set -euo pipefail

REPO="${REPO:-$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || echo "blaze-uz/app-orchestrator")}"
WIKI_URL="https://github.com/${REPO}.wiki.git"
TMPDIR="$(mktemp -d -t app-orch-wiki-XXXXXX)"

echo "Seeding wiki for $REPO from docs/ into $TMPDIR"

git clone "$WIKI_URL" "$TMPDIR" 2>&1 | tail -5

mapping=(
  "docs/README.md:Home.md"
  "docs/getting-started.md:Getting-Started.md"
  "docs/configuration.md:Configuration.md"
  "docs/http-api.md:HTTP-API.md"
  "docs/deployments.md:Deploy-Pipelines.md"
  "docs/ssh-remote-machines.md:Remote-Machines.md"
  "docs/troubleshooting.md:Troubleshooting.md"
  "docs/ARCHITECTURE.md:Architecture.md"
)

for entry in "${mapping[@]}"; do
  src="${entry%%:*}"
  dst="${entry##*:}"
  if [ -f "$src" ]; then
    cp "$src" "$TMPDIR/$dst"
    echo "  $src -> $dst"
  fi
done

# A tiny sidebar with the same nav order as docs/README.md
cat > "$TMPDIR/_Sidebar.md" <<'EOF'
- [Home](Home)
- [Getting Started](Getting-Started)
- [Configuration](Configuration)
- [HTTP API](HTTP-API)
- [Deploy Pipelines](Deploy-Pipelines)
- [Remote Machines](Remote-Machines)
- [Troubleshooting](Troubleshooting)
- [Architecture](Architecture)
EOF

cat > "$TMPDIR/_Footer.md" <<'EOF'
[Repo](https://github.com/blaze-uz/app-orchestrator) ·
[Releases](https://github.com/blaze-uz/app-orchestrator/releases) ·
[Issues](https://github.com/blaze-uz/app-orchestrator/issues) ·
[Security policy](https://github.com/blaze-uz/app-orchestrator/blob/main/SECURITY.md)
EOF

cd "$TMPDIR"
git add -A
if git diff --cached --quiet; then
  echo "Wiki already up to date."
else
  git commit -m "Seed wiki from docs/"
  git push origin master 2>/dev/null || git push origin main
  echo "Pushed $(git log --oneline -1)"
fi

echo "Done. View at https://github.com/${REPO}/wiki"
