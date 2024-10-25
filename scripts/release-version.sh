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

readme_pattern='\(untitaker\/hyperlink[@:]\)'
sed -i.bak "s/$readme_pattern$current_version/\\1$new_version/" README.md
rm README.md.bak
sed -i.bak "s/$readme_pattern$current_version/\\1$new_version/" .github/workflows/install-tester.yml
rm .github/workflows/install-tester.yml.bak
sed -i.bak "s/version = \"$current_version\"/version = \"$new_version\"/" Cargo.toml
rm Cargo.toml.bak

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
echo "  git push"
echo "  git push origin $new_version"
echo "  uncheck and check 'Publish to Marketplace' property of the new release"
echo "    see https://github.com/github/feedback/discussions/7941"
