+++
title = "Market identifiers"
description = "Typed MaLo-ID and EIC with their check characters verified at the parse, the 33-character MeLo-ID and the BDEW Marktpartner-ID — why an identifier that is a String is validated nowhere."
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

// The MeLo-ID **is** the Zählpunktbezeichnung — one identifier, one type.
// It has no check digit, so structure is what can be enforced: 33
// characters, country code, six-digit Netzbetreiber number.
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

## `Eic` — the Energy Identification Code

The sixteen-character ENTSO-E identifier: the one a Bilanzkreis, a
Bilanzierungsgebiet, a Regelzone and a Metering Grid Area are addressed by. It
shares nothing with a `BdewCode` — that is thirteen digits and addresses a
*Marktpartner*; an EIC is sixteen alphanumerics and addresses whatever its type
letter says.

ENTSO-E *EIC Reference Manual* §5.2–5.3:

| Position | Content |
|---|---|
| 1–2 | the Local Issuing Office, assigned by the Central Issuing Office |
| 3 | the object type — `X` party, `Y` area, `Z` measurement point, `W` resource object, `T` tie-line, `V` location, `A` substation |
| 4–15 | twelve characters assigned by the LIO |
| 16 | the check character |

```rust
use metering::ids::{Eic, EicType};

// The 50Hertz control area — a type `Y` code.
let regelzone: Eic = "10YDE-VE-------2".parse()?;
assert_eq!(regelzone.object_type(), Some(EicType::Area));
assert_eq!(regelzone.issuing_office(), "10");

// A transposed pair is a different, plausible-looking code — and is caught.
assert!("10YED-VE-------2".parse::<Eic>().is_err());
# Ok::<(), metering::ParseError>(())
```

### The check character *is* enforced

Unlike the Marktpartner-ID, the EIC scheme has no GS1-shaped carve-out: every
code the CIO or a LIO issues satisfies the algorithm. So a mistyped EIC is
rejected at the parse, the same treatment `MaloId` gets.

Each of the first fifteen characters takes a value (`0`–`9` → 0–9, `A`–`Z` →
10–35, `-` → 36), is multiplied by a weight running 16 down to 2, and the
products are summed. The check character is the one whose value is
`36 − ((Σ − 1) mod 37)`. A result of 36 would be a minus sign, which §5.2
forbids as a check character, so such a code is never issued —
`compute_check_character` answers `None` rather than writing one.

The manual prints two worked examples, `10X168Y4E6H0041Z` and
`10X---ENTSOE---L`; the test suite pins both, alongside the four German
Regelzonen and the German bidding zone.

### What is reported rather than enforced

The **issuing office** comes back as a `&str`, not a typed enum: the CIO
assigns these, publishes the list, and adds to it, so a closed set here would
reject codes from an office registered after this crate was released. The
manual says only *"the 2-characters identifying the LIO"*, so they are not even
required to be digits — though every office assigned so far uses two.

The **object type** is an `Option<EicType>` for the same reason: the type list
is ENTSO-E's to extend, and a library that hard-fails on a letter added after
its release rejects data the market has already issued. The *shape* is still
enforced — position 3 must be an uppercase letter.

### The German Bildungsvorschrift, and the Regelzone in position 4

The BDEW *Anwendungshilfe Energy Identification Codes* v1.0 (18.12.2017) §2.2.1
prints the same structure for the German market and pins the issuing office:

> Die Stellen 1 und 2 beschreiben die Vergabestelle: die Zahl „11" steht für das
> deutsche LIO im Strommarkt, den BDEW Bundesverband der Energie- und
> Wasserwirtschaft e.V.

§2.2.2 then gives **Bilanzierungsgebiete** their own table — a `Y` code in the
EIC function *Metering Grid Area* — in which position 4 carries a meaning no
other EIC has: it identifies the Regelzone.

| Position 4 | Regelzone |
|---|---|
| `N` | TenneT TSO |
| `R` | Amprion |
| `V` | 50Hertz Transmission |
| `W` | TransnetBW |

An EIC *function* is registry metadata and is not encoded in the code, so a `Y`
code cannot in general be told apart from a Bilanzkreis's. This one can, because
the same section adds a Praxishinweis: the Energie Codes und Services GmbH
**excludes** `N`, `R`, `V` and `W` at position 4 for every other `Y` function.

```rust
use metering::ids::{Eic, Regelzone};

// A Bilanzierungsgebiet in the Amprion Regelzone.
let bg: Eic = "11YR-AMPRION-BG9".parse()?;
assert_eq!(bg.regelzone(), Some(Regelzone::Amprion));
assert!(bg.is_german());

// The Regelzone's own ENTSO-E control-area code is a different code, issued
// by the CIO under LIO 10 — and its body still carries the pre-2010 company
// name, which is why it is worth having as a constant.
assert_eq!(
    Regelzone::Amprion.control_area_eic().to_string(),
    "10YDE-RWENET---I",
);
assert_eq!(Regelzone::Amprion.control_area_eic().regelzone(), None);
# Ok::<(), metering::ParseError>(())
```

### Where the EIC is required

`MeasurementPoint::bilanzkreis` and `MeasurementPoint::bilanzierungsgebiet` are
`Option<Eic>`, so a MaBiS Summenzeitreihe groups on a check-character-validated
value rather than on free text.

`EicType` is deliberately **not** constrained on either. A
Bilanzkreisverantwortlicher receives a `Y` code in the EIC function *Balance
Group*; older Bilanzkreise carry an `X` code, a national usage the ENTSO-E
manual explicitly keeps valid for Germany. Refusing one of the two would reject
identifiers the market issued.
