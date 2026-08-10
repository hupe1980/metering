# metering — task runner (https://just.systems)
#
# `just` with no arguments lists every recipe.

set shell := ["bash", "-uc"]

# MSRV — keep in sync with `rust-version` in Cargo.toml and rust-toolchain.toml.
msrv := "1.94"

# 📋 List all recipes
default:
    @just --list

# ✅ Everything CI runs, in CI order
ci: fmt-check lint purity test example doc package
    @echo "✅ all checks passed"

# 🧊 Enforce the "zero I/O, no clock" guarantee over non-comment source lines
purity:
    #!/usr/bin/env bash
    set -uo pipefail
    hits="$(grep -rn --include='*.rs' -E \
        'now_utc|SystemTime::now|Instant::now|std::(fs|env|net|process)|\bunsafe\b' \
        src/ | grep -vE ':[[:space:]]*(///|//!|//)' || true)"
    if [ -n "$hits" ]; then
        echo "❌ ambient state or unsafe reached the source:" >&2
        echo "$hits" >&2
        echo "" >&2
        echo "This crate promises equal inputs give equal outputs. Take the" >&2
        echo "timestamp as a parameter instead — see the Determinism section." >&2
        exit 1
    fi
    echo "🧊 pure: no clock, no I/O, no unsafe"

# 🎨 Format the workspace
fmt:
    cargo fmt --all

# 🎨 Fail if anything is unformatted
fmt-check:
    cargo fmt --all -- --check

# 📎 Clippy with warnings denied (default + all features)
lint:
    cargo clippy --all-targets -- -D warnings
    cargo clippy --all-targets --all-features -- -D warnings

# 🔎 Fast type-check, all features
check:
    cargo check --all-targets --all-features

# 🧪 Full test suite (all features, incl. doctests)
test:
    cargo test --all-features

# ▶️  Run the end-to-end pipeline example
example:
    cargo run --all-features --example pipeline

# 🧪 Run tests matching a filter, e.g. `just test-one gas_m3`
test-one filter:
    cargo test --all-features {{ filter }} -- --nocapture

# 🎛️ Build every feature combination
features:
    cargo build --no-default-features
    cargo build --no-default-features --features serde
    cargo build --all-features

# 🦀 Compile on the pinned MSRV
msrv:
    RUSTUP_TOOLCHAIN={{ msrv }} cargo check --all-features --all-targets

# 📚 Build the docs with warnings denied
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

# 📚 Build and open the docs
doc-open:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --open

# 📦 Dry-run the crates.io package (catches bad metadata before tagging)
# `--allow-dirty` is local-only convenience; CI runs this on a clean checkout.
package:
    cargo publish --dry-run --all-features --allow-dirty

# 🛡️ Audit dependencies for advisories (needs `cargo install cargo-audit`)
audit:
    cargo audit

# ⬆️ Show outdated dependencies (needs `cargo install cargo-outdated`)
outdated:
    cargo outdated --root-deps-only

# 🏷️ Tag the current Cargo.toml version and push it — triggers release.yml
tag:
    #!/usr/bin/env bash
    set -euo pipefail
    version="$(cargo metadata --no-deps --format-version 1 \
        | grep -o '"version":"[^"]*"' | head -1 | cut -d'"' -f4)"
    if [ -n "$(git status --porcelain)" ]; then
        echo "❌ working tree is dirty — commit first" >&2
        exit 1
    fi
    just ci
    git tag -a "v${version}" -m "v${version}"
    echo "🏷️  tagged v${version} — push with: git push origin v${version}"

# 🌐 Serve the documentation site locally (needs `zola`)
site:
    zola --root site serve

# 🌐 Build the site and validate every internal link
site-build:
    zola --root site check
    zola --root site build --output-dir public --force

# 🧹 Remove build artifacts
clean:
    cargo clean
    rm -rf site/public
