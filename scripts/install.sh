#!/bin/sh
set -e
tag="`grep 'version = ' Cargo.toml | head -1 | cut -d'"' -f2`"

echo "downloading hyperlink $tag"

# Download the installer to a file and verify it against its GitHub build
# attestation before running it (see #198). Set HYPERLINK_SKIP_ATTESTATION=1 to
# skip — e.g. on runners without `gh` / a GitHub token, such as Forgejo.
installer="`mktemp`"
trap 'rm -f "$installer"' EXIT

curl --proto '=https' --tlsv1.2 -LsSf \
    "https://github.com/untitaker/hyperlink/releases/download/$tag/hyperlink-installer.sh" \
    -o "$installer"

if [ -z "$HYPERLINK_SKIP_ATTESTATION" ]; then
    # --signer-workflow pins the producing workflow, not just the repo.
    gh attestation verify "$installer" \
        --repo untitaker/hyperlink \
        --signer-workflow untitaker/hyperlink/.github/workflows/release.yml
else
    echo "hyperlink: HYPERLINK_SKIP_ATTESTATION set, skipping attestation check" >&2
fi

sh "$installer"
