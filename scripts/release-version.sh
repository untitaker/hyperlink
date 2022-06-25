#!/bin/bash
set -xe

[ -z "$(git status --porcelain)" ] || (echo "dirty working directory" && exit 1)

current_version="$(grep '^version = ' Cargo.toml | head -1 | cut -d '"' -f2)"
new_version="$1"

if [ -z "$new_version" ]; then
    echo "New version required as argument"
    exit 1
fi

echo ">>> Bumping version"

action_prefix="untitaker\\/hyperlink@"
sed -i.bak "s/$action_prefix$current_version/$action_prefix$new_version/" README.md
rm README.md.bak
sed -i.bak "s/version = \"$current_version\"/version = \"$new_version\"/" Cargo.toml
rm Cargo.toml.bak
sed -i.bak "s/\"version\": \"$current_version\"/\"version\": \"$new_version\"/" package.json
rm package.json.bak

echo ">>> Running tests"
cargo build
cargo test

echo ">>> Commit"

git add README.md
git add Cargo.toml
git commit -am "version $new_version"
git tag $new_version

git show HEAD

set +x

echo "things left to do:"
echo "  cargo publish"
echo "  npm publish"
echo "  git push"
echo "  git push origin $new_version"
echo "  uncheck and check 'Publish to Marketplace' property of the new release"
echo "    see https://github.com/github/feedback/discussions/7941"
