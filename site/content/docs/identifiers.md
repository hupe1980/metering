+++
title = "Market identifiers"
description = "Typed MaLo-ID with the BDEW check digit verified at the parse, and the 33-character MeLo-ID — why an identifier that is a String is validated nowhere."
weight = 4
+++

An identifier that is a `String` is validated nowhere. A transposed digit in a
Marktlokations-ID becomes a *different, plausible-looking* MaLo-ID — a wrong
database key, a reading filed against the wrong delivery point, and no error
anywhere. The MaLo-ID carries a **check digit** precisely so that this class of
mistake is detectable; a type that does not check it throws that protection
away.

```rust
use metering::{MaloId, MeloId};

// The worked example from the BDEW Anwendungshilfe (v1.0, 28.04.2017).
let malo: MaloId = "41373559241".parse()?;
assert_eq!(malo.check_digit(), 1);

// A transposed digit no longer matches its check digit.
assert!("41373559214".parse::<MaloId>().is_err());

// The MeLo-ID / Zählpunktbezeichnung has no check digit, so structure is
// what can be enforced: 33 characters, country code, six-digit NB number.
let melo: MeloId = "DE00056266802AO6G56M11SN51G21M24S".parse()?;
assert_eq!(melo.netzbetreiber_nr(), "000562");
# Ok::<(), metering::ParseError>(())
```

## The check digit, from the primary source

The BDEW Anwendungshilfe *"Die neue Marktlokations-Identifikationsnummer"*
defines the Bildungsvorschrift — eleven digits, first digit `1`–`3` for the
DVGW and `4`–`9` for the BDEW as Vergabestelle — and the check-digit schema,
with a worked example the test suite reproduces digit for digit:

```text
4 1 3 7 3 5 5 9 2 4 →  a) odd positions:        4+3+3+5+2 = 17
                        b) even positions × 2:  (1+7+5+9+4) × 2 = 52
                        c) 17 + 52 = 69
                        d) next multiple of 10: 70 − 69 = 1  →  41373559241
```

Note step b doubles the **sum**, not each digit — this is *not* the Luhn
algorithm. One consequence, documented rather than glossed over: the scheme
misses a ±5 change in an even position, which shifts the total by exactly 10.
The property tests pin both what the scheme catches and what it cannot.

## Where the types are required

`MeasurementPoint`, `MeasurementSeries` and the meter lifecycle events carry
`MaloId` / `MeloId`, so an invalid identifier fails at the boundary — where
the message it arrived in is still available to report.

The keys of a virtual-meter `SourceMap` deliberately stay `String`: they are
arbitrary series labels a caller chooses (`"PLANT"`, `"T2"`), not asserted to
be MaLo-IDs.

## `BdewCode` — the Marktpartner-ID

The 13-digit BDEW- or DVGW-Codenummer every market participant is addressed by:
`NAD+MS`/`NAD+MR` in MSCONS, the Marktpartner segments in UTILMD, and the number
on every Netzbetreiber's price sheet. BDEW *Identifikatoren in der
Marktkommunikation* v1.2 §2.2 prints the Bildungsvorschrift:

| Position | Content |
|---|---|
| 1–2 | Vergabestelle/Sparte — `99` BDEW/Strom, `98` DVGW/Gas |
| 3 | `0`–`8` for BDEW, `0`–`9` for DVGW |
| 4–12 | digits `0`–`9` |
| 13 | Prüfziffer |

```rust
use metering::{BdewCode, CodeVergabestelle};

let nb: BdewCode = "9900987654321".parse()?;
assert_eq!(nb.vergabestelle(), CodeVergabestelle::BdewStrom);
```

`MeasurementPoint::accountable_mp_id` and `MeasurementSource::Mscons`'s
`sender_mp_id` carry it, and `Zaehlzeitdefinition::netzbetreiber` says whose
tariff calendar a definition is.

### The check digit is verified, but not enforced

§2.3 says the Prüfziffer uses the **same** Lok- und
Waggon-Kennzeichnungsverfahren as the MaLo-ID — and carves out an exception in
the very next sentence:

> Bei einer von GS1 vergebenen GLN (= Globale Lokationsnummer) gilt das von GS1
> verwendete Prüfzifferverfahren.

So a perfectly well-formed Marktpartner-ID may legitimately fail the BDEW
procedure. `BdewCode` parses it anyway and reports the outcome through
`has_bdew_check_digit()` instead, so a caller can warn without a library
refusing data the market issued.

`MaloId` is the other way round — checked at the parse, rejecting a mismatch —
because its Bildungsvorschrift has no such carve-out and its worked example is
printed in the same document. The asymmetry is the point: enforce what can be
verified end to end, report what cannot.
