#!/bin/sh
set -e
tag="`grep 'version = ' Cargo.toml | head -1 | cut -d'"' -f2`"

echo "downloading hyperlink $tag"

# Download the installer to a file and verify it against its GitHub build
# attestation before running it (see #198). Verification needs the `gh` CLI; when
# `gh` is absent (e.g. Forgejo or other non-GitHub CI) the check is skipped with
# a warning. Set HYPERLINK_SKIP_ATTESTATION=1 to skip it entirely, or
# HYPERLINK_FORCE_ATTESTATION=1 to fail instead of skipping when `gh` is missing.
installer="`mktemp`"
trap 'rm -f "$installer"' EXIT

curl --proto '=https' --tlsv1.2 -LsSf \
    "https://github.com/untitaker/hyperlink/releases/download/$tag/hyperlink-installer.sh" \
    -o "$installer"

if [ -n "$HYPERLINK_SKIP_ATTESTATION" ]; then
    echo "hyperlink: HYPERLINK_SKIP_ATTESTATION set, skipping attestation verification" >&2
elif command -v gh >/dev/null 2>&1; then
    # --signer-workflow pins the producing workflow, not just the repo.
    gh attestation verify "$installer" \
        --repo untitaker/hyperlink \
        --signer-workflow untitaker/hyperlink/.github/workflows/release.yml
elif [ -n "$HYPERLINK_FORCE_ATTESTATION" ]; then
    echo "hyperlink: HYPERLINK_FORCE_ATTESTATION is set but 'gh' is not installed; cannot verify attestation" >&2
    exit 1
else
    echo "hyperlink: 'gh' not found, skipping attestation verification (set HYPERLINK_FORCE_ATTESTATION=1 to require it)" >&2
fi

sh "$installer"
