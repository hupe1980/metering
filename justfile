# metering — task runner (https://just.systems)
#
# `just` with no arguments lists every recipe.

set shell := ["bash", "-uc"]

# MSRV — read from `rust-version` in Cargo.toml, its one source.
msrv := `sed -n 's/^rust-version *= *"\(.*\)"/\1/p' Cargo.toml`

# Tests read fixture files and time themselves; the purity bans in clippy.toml
# hold for the library alone, which `purity` checks.
impure_ok := "-A clippy::disallowed_methods -A clippy::disallowed_types"

# 📋 List all recipes
default:
    @just --list

# ✅ The CI gate, exactly — the `gate` job in .github/workflows/ci.yml runs `just ci`
ci: fmt-check lint purity test doc example package deny
    @echo "✅ all checks passed"

# 🧊 No clock, no env, no I/O in the library: clippy.toml's disallowed methods/types
purity:
    cargo clippy --lib --all-features -- -D warnings

# 🎨 Format the workspace
fmt:
    cargo fmt --all

# 🎨 Fail if anything is unformatted
fmt-check:
    cargo fmt --all -- --check

# 📎 Clippy with warnings denied (default + all features)
lint:
    cargo clippy --all-targets -- -D warnings {{ impure_ok }}
    cargo clippy --all-targets --all-features -- -D warnings {{ impure_ok }}

# 🔎 Fast type-check, all features
check:
    cargo check --all-targets --all-features

# 🧪 Test suite with default (= no) features and with all features, incl. doctests
test:
    cargo test
    cargo test --all-features

# ▶️  Run the end-to-end pipeline example (it asserts its own invariants)
example:
    cargo run --all-features --example pipeline

# 🧪 Run tests matching a filter, e.g. `just test-one gas_m3`
test-one filter:
    cargo test --all-features {{ filter }} -- --nocapture

# 📚 Build the docs with warnings denied
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

# 📚 Build and open the docs
doc-open:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --open

# `--allow-dirty` matters only locally; CI checks out clean.
#
# 📦 Dry-run the crates.io package (catches bad metadata before tagging)
package:
    cargo publish --dry-run --all-features --allow-dirty

# 🛡️ Advisories, licenses, duplicate crates, sources (needs `cargo install cargo-deny`)
deny:
    cargo deny check

# 🦀 Run the whole test suite on the MSRV (`rust-version` in Cargo.toml)
msrv:
    RUSTUP_TOOLCHAIN={{ msrv }} cargo test --all-features

# 📉 Test against the lowest versions Cargo.toml admits (needs a nightly toolchain)
minimal-versions:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p target && cp Cargo.lock target/Cargo.lock.keep
    trap 'mv target/Cargo.lock.keep Cargo.lock' EXIT
    cargo +nightly update -Z direct-minimal-versions
    cargo test --all-features

# 🧬 Mutation-test the code changed since `base` (needs `cargo install cargo-mutants`)
mutants base="main":
    mkdir -p target && git diff {{ base }}... > target/mutants.diff
    cargo mutants --all-features --in-diff target/mutants.diff

# Needs `cargo install cargo-semver-checks`. `--release-type patch` makes it
# list every break; inferred from a 0.x minor bump, it skips every lint.
#
# 🔒 What would break for a consumer on the last published version
semver:
    cargo semver-checks check-release --all-features --release-type patch

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

# Needs `just references` and `pdftotext` (poppler); extra corpora can be
# passed as arguments. Not a CI job: the corpus is gitignored.
#
# 📚 Check every German quote in src/, site/ and README.md against the PDFs
quotes *dirs:
    python3 scripts/verify_quotes.py {{ dirs }}

# Cross-checks `concepts/` and `specs/` (rules in the script's docstring).
# Not a CI job: both folders are gitignored.
#
# 🗺️ Check the design notes and the specifications against each other
_concepts-check:
    python3 scripts/check-concepts.py

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

# Gitignored third-party publications. Keeps files already on disk, so a rerun
# is safe; what cannot be fetched is listed at the end.
#
# 📚 Rebuild `concepts/reference/` — the primary sources every citation is checked against
references:
    #!/usr/bin/env bash
    set -uo pipefail
    ua='Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36'
    missing=""
    # fetch DIR FILE URL [ALT_URL]
    fetch() {
        mkdir -p "concepts/reference/$1"
        if [ -s "concepts/reference/$1/$2" ]; then echo "kept     $1/$2"; return 0; fi
        for url in "$3" "${4:-}"; do
            [ -n "$url" ] || continue
            if curl -fsSL -A "$ua" --retry 3 --retry-delay 5 --max-time 900 \
                    -o "concepts/reference/$1/$2.part" "$url" \
                    && [ -s "concepts/reference/$1/$2.part" ] \
                    && [ "$(file -b --mime-type "concepts/reference/$1/$2.part")" != "text/html" ]; then
                mv "concepts/reference/$1/$2.part" "concepts/reference/$1/$2"; echo "fetched  $1/$2"; return 0
            fi
            rm -f "concepts/reference/$1/$2.part"
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
    fetch law eeg.pdf 'https://www.gesetze-im-internet.de/eeg_2014/EEG_2023.pdf'
    fetch law enfg.pdf 'https://www.gesetze-im-internet.de/enfg/EnFG.pdf'
    fetch law bgbl-2025-i-347-enwg-novelle-20251222.pdf \
        'https://www.recht.bund.de/bgbl/1/2025/347/regelungstext.pdf?__blob=publicationFile&v=2'
    # bnetza/ — the Festlegungen the repealed ordinances left behind
    fetch bnetza bk6-23-241-beschluss-20260507.pdf \
        'https://www.bundesnetzagentur.de/DE/Beschlusskammern/1_GZ/BK6-GZ/2023/BK6-23-241/BK6-23-241_beschluss_vom_07.05.26.pdf?__blob=publicationFile&v=1'
    fetch bnetza bk6-23-241-anlage-bilarem.pdf \
        'https://www.bundesnetzagentur.de/DE/Beschlusskammern/1_GZ/BK6-GZ/2023/BK6-23-241/BK6-23-241_bilarem.pdf?__blob=publicationFile&v=1'
    fetch bnetza bk7-24-01-008-gabi-gas-2-1-beschluss.pdf \
        'https://www.bundesnetzagentur.de/DE/Beschlusskammern/1_GZ/BK7-GZ/2024/BK7-24-0008/Anlagen/BK7-24-01-0008_Beschluss_DL_BF.pdf?__blob=publicationFile&v=5'
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
    fetch edi-energy utilts-ahb-1.1.pdf \
        'https://www.bdew-mako.de/api/downloadFile/12230'
    fetch edi-energy utilts-mig-1.1e.pdf \
        'https://www.bdew-mako.de/api/downloadFile/10706'
    # MiSpeL (Az. 618-25-02), adopted 01.10.2026: Tenor and both Anlagen
    mispel='https://www.bundesnetzagentur.de/DE/Fachthemen/ElektrizitaetundGas/ErneuerbareEnergien/EEG_Aufsicht/MiSpeL/DL'
    fetch bnetza mispel-tenor-20261001.pdf "$mispel/MiSpeL_TenorMitBegruendung.html?nn=1067830"
    fetch bnetza mispel-anlage1-abgrenzungsoption-20261001.pdf "$mispel/MiSpeL_Abgrenzungsoptionen.html?nn=1067830"
    fetch bnetza mispel-anlage2-pauschaloption-20261001.pdf "$mispel/MiSpeL_Pauschaloptionen.html?nn=1067830"
    # bdew/ — Anwendungshilfen and the gas-SLP Leitfaden
    fetch bdew bdew-slp-strom-2025-profile-h25-g25-l25-p25-s25.xlsx \
        'https://www.bdew.de/media/documents/Kopie_von_Repr%C3%A4sentative_Profile_BDEW_H25_G25_L25_P25_S25_Ver%C3%B6ffentlichung.xlsx'
    fetch bdew vdew-repraesentative-lastprofile-1999.pdf \
        'https://www.bdew.de/media/documents/1999_Repraesentative-VDEW-Lastprofile.pdf'
    fetch bdew vdew-lastprofile-step-by-step-2000.pdf \
        'https://www.bdew.de/media/documents/2000131_Anwendung-repraesentativen_Lastprofile-Step-by-step.pdf'
    fetch bdew bdew-lf-ausfallarbeit-redispatch-2-0-202005.pdf \
        'https://www.bdew.de/media/documents/Awh_2020-05_RD_2.0_LF_Ausfallarbeit.pdf'
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
    # The EDI@Energy Anwendungshilfe carries the worked § 42b formulas.
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
        echo "⚠️  not fetched (see concepts/REFERENCES.md for the source):" >&2
        printf '%s' "$missing" >&2
    fi
    echo "📚 concepts/reference/ rebuilt"
