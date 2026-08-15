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
