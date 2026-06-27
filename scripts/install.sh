#!/bin/sh
set -e
tag="`grep 'version = ' Cargo.toml | head -1 | cut -d'"' -f2`"

echo "downloading hyperlink $tag"

# Download the installer to a file (rather than piping straight into sh) and
# verify it against its GitHub build attestation before running it. The
# attestation is Sigstore-backed and lives in GitHub's transparency log, so —
# unlike a checksum shipped beside the script in the same mutable release — it
# cannot be swapped together with the artifact. See #198.
#
# Requires `gh` (preinstalled on GitHub runners) and a token in GH_TOKEN /
# GITHUB_TOKEN. Attestations exist for releases built after
# `github-attestations` was enabled in dist-workspace.toml.
installer="`mktemp`"
# Always clean up the temp file, including when `set -e` aborts on a failed
# verification below.
trap 'rm -f "$installer"' EXIT

curl --proto '=https' --tlsv1.2 -LsSf \
    "https://github.com/untitaker/hyperlink/releases/download/$tag/hyperlink-installer.sh" \
    -o "$installer"

# Pin the producing workflow, not just the repo: only an attestation minted by
# the real release workflow is accepted, so a different (or compromised)
# workflow in the same repo cannot mint one that passes.
gh attestation verify "$installer" \
    --repo untitaker/hyperlink \
    --signer-workflow untitaker/hyperlink/.github/workflows/release.yml

sh "$installer"
