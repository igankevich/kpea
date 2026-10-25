#!/bin/sh

. ./ci/preamble.sh

main() {
    update_cargo_fast_install
    create_tag
    create_release
}

update_cargo_fast_install() {
    git config user.name "$GITHUB_ACTOR via GH actions"
    git config user.email github-actions@github.com
    git add cli/cargo-fast-install.jsonl
    if ! git diff --staged --exit-code >/dev/null; then
        git commit -m "Update \`cli/cargo-fast-install.jsonl\` for release $GITHUB_REF_NAME"
        git push
    fi
}

create_tag() {
    version="$(
        python3 <<'EOF'
import subprocess
import json

output = subprocess.run(["cargo", "metadata", "--format-version=1", "--no-deps"], capture_output=True, text=True, check=True)
print(json.loads(output.stdout)["packages"][0]["version"])
EOF
    )"
    if test -n "$(git tag --list "$version")"; then
        if test "$(git rev-list -n 1 "$version")" = "$(git rev-parse HEAD)"; then
            return
        fi
        printf "Git tag %s already exists\n" "$version" >&2
        exit 1
    fi
    git tag "$version"
    git push --tags
}

create_release() {
    gh release create "$version" \
        --draft \
        --generate-notes \
        --title "Release $version" \
        kpea*tar.gz
}

main
