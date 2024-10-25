#!/bin/sh
set -e
tag="`grep 'version = ' Cargo.toml | head -1 | cut -d'"' -f2`"

echo "downloading hyperlink $tag"

curl --proto '=https' --tlsv1.2 -LsSf https://github.com/untitaker/hyperlink/releases/download/$tag/hyperlink-installer.sh | sh
