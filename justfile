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

# Verify every passage the crate presents as verbatim source text.
#
# Needs `specs/` (run `just specs`) and `pdftotext` (poppler). Extra corpora can
# be passed as arguments — the sibling workspaces hold documents this crate
# cites but does not mirror.
#
# Not a CI lane: the corpus is gitignored, so a fresh checkout has nothing to
# check against. Run it every audit round, and after touching a quote.
#
# 📚 Check every German quote in src/, site/ and README.md against the PDFs
quotes *dirs:
    python3 scripts/verify_quotes.py {{ dirs }}

# 🌐 Serve the documentation site locally (needs `zola`)
site:
    zola --root site serve

# 🌐 Build the site and validate every internal link
site-build:
    zola --root site check
    zola --root site build --force

# 🧹 Remove build artifacts
clean:
    cargo clean
    rm -rf site/public

# Third-party publications: gitignored, never committed. Keeps whatever is
# already on disk, so a partial run is safe to repeat. What cannot be fetched
# is reported at the end and indexed in `specs/README.md` with its source.
#
# 📚 Rebuild `specs/` — the primary sources every citation is checked against
specs:
    #!/usr/bin/env bash
    set -uo pipefail
    ua='Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36'
    missing=""
    # fetch DIR FILE URL [ALT_URL]
    fetch() {
        mkdir -p "specs/$1"
        if [ -s "specs/$1/$2" ]; then echo "kept     $1/$2"; return 0; fi
        for url in "$3" "${4:-}"; do
            [ -n "$url" ] || continue
            if curl -fsSL -A "$ua" --retry 3 --retry-delay 5 --max-time 900 \
                    -o "specs/$1/$2.part" "$url" \
                    && [ -s "specs/$1/$2.part" ] \
                    && [ "$(file -b --mime-type "specs/$1/$2.part")" != "text/html" ]; then
                mv "specs/$1/$2.part" "specs/$1/$2"; echo "fetched  $1/$2"; return 0
            fi
            rm -f "specs/$1/$2.part"
        done
        echo "MISSING  $1/$2  <- $3" >&2
        missing="$missing  $1/$2  <- $3"$'\n'
        return 0
    }
    # law/ — the statutes, consolidated (gesetze-im-internet.de)
    fetch law enwg.pdf 'https://www.gesetze-im-internet.de/enwg_2005/EnWG.pdf'
    fetch law msbg.pdf 'https://www.gesetze-im-internet.de/messbg/MsbG.pdf'
    fetch law messeg.pdf 'https://www.gesetze-im-internet.de/messeg/MessEG.pdf'
    fetch law messev.pdf 'https://www.gesetze-im-internet.de/messev/MessEV.pdf'
    fetch law stromnev.pdf 'https://www.gesetze-im-internet.de/stromnev/StromNEV.pdf'
    fetch law heizkostenv.pdf 'https://www.gesetze-im-internet.de/heizkostenv/HeizkostenV.pdf'
    fetch law bgbl-2025-i-347-enwg-novelle-20251222.pdf \
        'https://www.recht.bund.de/bgbl/1/2025/347/regelungstext.pdf?__blob=publicationFile&v=2'
    # bnetza/ — the Festlegungen the repealed ordinances left behind
    fetch bnetza bk6-22-300-beschluss-20231127.pdf \
        'https://www.bundesnetzagentur.de/DE/Beschlusskammern/1_GZ/BK6-GZ/2022/BK6-22-300/Beschluss/BK6-22-300_Beschluss_20231127.pdf?__blob=publicationFile&v=1'
    fetch bnetza bk6-22-300-anlage1-20231127.pdf \
        'https://www.bundesnetzagentur.de/DE/Beschlusskammern/1_GZ/BK6-GZ/2022/BK6-22-300/Beschluss/BK6-22-300_Beschluss_Anlage1.pdf?__blob=publicationFile&v=1'
    fetch bnetza bk6-22-300-vde-fnn-empfehlung-tenorziffer-2f.pdf \
        'https://www.bundesnetzagentur.de/DE/Beschlusskammern/1_GZ/BK6-GZ/2022/BK6-22-300/Mitteilung/Mitteilung_3/VDE_FNN_Empfehlung_zu_Tenorziffer_2f.pdf?__blob=publicationFile&v=1'
    fetch bnetza bk6-24-174-beschluss-20241024.pdf \
        'https://www.bundesnetzagentur.de/DE/Beschlusskammern/1_GZ/BK6-GZ/2024/BK6-24-174/Beschluss/BK6-24-174_Beschluss_vom_20241024.pdf?__blob=publicationFile&v=1'
    fetch bnetza bk6-24-174-gpke-teil1-lesefassung.pdf \
        'https://www.bundesnetzagentur.de/DE/Beschlusskammern/1_GZ/BK6-GZ/2024/BK6-24-174/Beschluss/BK6-24-174_GPKE_Teil1_Lesefassung.pdf?__blob=publicationFile&v=1'
    # edi-energy/ — the BDEW catalogue, one file per fileId
    fetch edi-energy codeliste-obis-kennzahlen-und-medien-2.5c.pdf \
        'https://www.bdew-mako.de/api/downloadFile/11918'
    fetch edi-energy allgemeine-festlegungen-6.1c.pdf \
        'https://www.bdew-mako.de/api/downloadFile/11916'
    fetch edi-energy allgemeine-festlegungen-6.1d.pdf \
        'https://www.bdew-mako.de/api/downloadFile/12145'
    fetch edi-energy codeliste-slp-tu-muenchen-1.1.pdf \
        'https://www.bdew-mako.de/api/downloadFile/9092'
    fetch edi-energy mscons-mig-2.4c.pdf \
        'https://www.bdew-mako.de/api/downloadFile/9645'
    fetch edi-energy mscons-mig-2.5.pdf \
        'https://www.bdew-mako.de/api/downloadFile/12175'
    # bdew/ — Anwendungshilfen and the gas-SLP Leitfaden
    fetch bdew bdew-lf-slp-gas-kov-xv-20260327.pdf \
        'https://www.bdew.de/media/documents/260327_LF_SLP_Gas_KoV_XV_CO4f7Rb.pdf'
    fetch bdew bdew-lf-slp-gas-anlage2-pruefroutine-siglinde.xlsm \
        'https://www.bdew.de/media/documents/20240322_KoV_XIV_LF-SLP_Anlage_2-Pr%C3%BCfroutine_Synthetisches_Verfahren_SigLinDe.xlsm'
    fetch bdew bdew-awh-slp-strom-2025-20250317.pdf \
        'https://www.bdew.de/media/documents/2025-03-17_AWH_Aktualisierte_SLP_Strom_2025_Ver%C3%B6ffentlichung.pdf'
    fetch bdew bdew-awh-modul-3-v1.1-20250207.pdf \
        'https://www.bdew.de/media/documents/BDEW-AWH_Modul_3_V1.1_Korrektur070225.pdf'
    fetch bdew bdew-awh-identifikatoren-mako-v1.2.pdf \
        'https://www.bdew.de/media/documents/AWH_Identifikatoren-in-der-Marktkommunikation_Version.1.2.pdf'
    fetch bdew bdew-awh-malo-id-v1.0-20170428.pdf \
        'https://bdew-codes.de/Content/Files/MaLo/2017-04-28-BDEW-Anwendungshilfe-MaLo-ID_Version1.0_FINAL.PDF' \
        'https://www.bundesnetzagentur.de/DE/Beschlusskammern/_SharedDocs/Mitteilungen_zu_BK6_16_200_BK7_16_142_/Mitteilung_Nr_2/Anlage_1_Anwendungshilfe_MaLo_ID.pdf?__blob=publicationFile&v=1'
    fetch bdew bdew-awh-eic-vergabe-v1.0-20171218.pdf \
        'https://bdew-codes.de/Content/Files/EIC/Awh_20171218_EIC-Vergabe_V1-0.pdf'
    # The EDI@Energy Anwendungshilfe carries the worked § 42b formulas; the BDEW
    # Anwendungshilfe zum Solarpaket I is no longer served as a PDF.
    fetch edi-energy awh-berechnungsformeln-solarpaket-1-v1.1.pdf \
        'https://www.bdew-mako.de/api/downloadFile/11113'
    # vde-fnn/ — what the paywalled Anwendungsregeln are cited through
    fetch vde-fnn vde-fnn-hinweis-bewertung-mindestleistung.pdf \
        'https://www.vde.com/resource/blob/2384818/09cfe30a1ef5a210bebf37ff14953858/vde-fnn-hinweis-bewertung-der-mindestleistung-data.pdf'
    fetch vde-fnn vde-fnn-hinweis-symmetrischer-anschluss-4100.pdf \
        'https://www.vde.com/resource/blob/2243242/931a9d5c1e48cf5c55592bcda7c59e20/fnn-hinweis-anforderungen-fuer-den-symmetrischen-anschluss-und-betrieb-nach-vde-ar-n-4100-data.pdf'
    # eu/ — the Gastag's own definition
    fetch eu vo-eu-312-2014-gasnetzkodex-bilanzierung-de.pdf \
        'https://eur-lex.europa.eu/legal-content/DE/TXT/PDF/?uri=CELEX:32014R0312'
    if [ -n "$missing" ]; then
        echo "" >&2
        echo "⚠️  not fetched (see specs/README.md for the source):" >&2
        printf '%s' "$missing" >&2
    fi
    echo "📚 specs/ rebuilt"
