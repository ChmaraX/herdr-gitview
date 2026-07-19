# herdr-gitview dev tasks

# Build release and remind how to link the dev copy into herdr.
dev:
    cargo build --release
    @echo "linked? if not: herdr plugin link $(pwd)"

test:
    cargo test

lint:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings

# Everything a release tag will run, locally.
release-dry: lint test
    cargo build --release
    @echo "ok — tag with: git tag v$(sed -n 's/^version *= *\"\\(.*\\)\"/\\1/p' herdr-plugin.toml | head -1) && git push --tags"
