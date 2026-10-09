+++
title = "Identifiers"
description = "MaLo-ID, MeLo-ID, BDEW-Codenummer, EIC and OBIS as types: parsed at the boundary, check characters verified, one canonical string each."
weight = 5
[extra]
group = "Concepts"
+++

Each identifier is a type that verifies what its scheme defines at the parse.
Parse at the boundary.

| Type | What | Check | Source |
|---|---|---|---|
| `MaloId` | Marktlokation, 11 digits | check digit, enforced | BDEW Anwendungshilfe *Die neue Marktlokations-Identifikationsnummer* v1.0, § 3.2 |
| `MeloId` | Messlokation = Zählpunktbezeichnung, 33 characters | structure only — there is no check digit | VDE-AR-N 4400 / DVGW G 2000 |
| `BdewCode` | Marktpartner-ID, 13 digits | BDEW or GS1 check digit, reported | BDEW *Identifikatoren in der Marktkommunikation* v1.2, § 2.2, § 6.1 |
| `Eic` | ENTSO-E Energy Identification Code, 16 characters | check character, enforced | EIC Reference Manual § 5.1 |
| `ObisCode` | measurement channel `A-B:C.D.E` | value groups | EDI@Energy *Codeliste der OBIS-Kennzahlen und Medien* 2.5c |

## MaLo-ID

The check digit is the *Lok- und Waggon-Kennzeichnungsverfahren*: the sum of
the odd positions plus **twice the sum** of the even positions (not Luhn), to
the next multiple of ten. It misses a ±5 change in an even position and a
transposition of two digits five apart.

```rust
use metering::MaloId;

// The worked example of the BDEW Anwendungshilfe.
let malo: MaloId = "41373559241".parse().unwrap();
assert_eq!(malo.check_digit(), 1);

// A transposed pair no longer matches its check digit.
assert!("41373559214".parse::<MaloId>().is_err());
```

## MeLo-ID

The MeLo-ID **is** the Zählpunktbezeichnung: country code, six-digit
Netzbetreiber number, five characters of Postleitzahl, twenty assigned by the
Netzbetreiber.

```rust
use metering::MeloId;

let melo: MeloId = "DE00056266802AO6G56M11SN51G21M24S".parse().unwrap();
assert_eq!(melo.country(), "DE");
assert_eq!(melo.netzbetreiber_nr(), "000562");
assert!("not a zaehlpunkt".parse::<MeloId>().is_err());
```

## BDEW-Codenummer

The thirteenth digit follows the MaLo procedure **except** for a GS1 GLN, which
carries GS1's own. The type accepts a code that passes either and reports which
(`issuer`, `has_bdew_check_digit`).

```rust
use metering::ids::{BdewCode, Issuer};

let check = BdewCode::compute_check_digit("990098765432").unwrap();
let nb: BdewCode = format!("990098765432{check}").parse().unwrap();
assert_eq!(nb.issuer(), Issuer::Bdew);

let gln: BdewCode = "4012345000009".parse().unwrap();
assert_eq!(gln.issuer(), Issuer::Gs1);

assert!("9900987654321".parse::<BdewCode>().is_err()); // fails both
```

## EIC

Addresses Bilanzkreise, Bilanzierungsgebiete and Regelzonen; the check
character is enforced. A German Bilanzierungsgebiet names its Regelzone in
position 4 (`Eic::regelzone`).

```rust
use metering::ids::{Eic, EicType, Regelzone};

let regelzone: Eic = "10YDE-VE-------2".parse().unwrap();
assert_eq!(regelzone.object_type(), Some(EicType::Area));
assert!("10YED-VE-------2".parse::<Eic>().is_err()); // transposed pair

assert_eq!(Regelzone::Amprion.control_area_eic().to_string(), "10YDE-RWENET---I");
assert_eq!(Eic::compute_check_character("10X---ENTSOE---"), Some('L'));
```

## OBIS codes

`A-B:C.D.E`: A the medium, C the quantity and **direction** (1 Bezug, 2
Lieferung), D the Messart (8 Zählerstand, 29 Lastgang, 6 Maximum), E the tariff
(63 is the Fehlerregister). Kind, unit, tariff and default grid derive from one
reading of the code.

```rust
use metering::{Direction, ObisCode, Resolution, Unit};

let bezug: ObisCode = "1-0:1.8.0*255".parse().unwrap();
assert_eq!(bezug.to_string(), "1-0:1.8.0"); // one canonical string
assert_eq!(bezug.direction(), Some(Direction::Import));
assert_eq!(bezug.unit(), Some(Unit::KiloWattHour));
assert_eq!(bezug.as_lastgang(), Some(ObisCode::STROM_BEZUG_LASTGANG));

// A Brennwert's E names the averaging period, not a tariff.
assert_eq!(ObisCode::GAS_BRENNWERT_MONATSMITTEL.tariff(), None);
assert_eq!(ObisCode::GAS_BRENNWERT_MONATSMITTEL.default_resolution(), Some(Resolution::Month));
```

`Display` writes the reduced form — Codeliste § 2.3: *"Wertegruppe F wird für
die Kommunikation im deutschen Gasmarkt nicht verwendet"*. `{:#}` writes all
six groups; parsing accepts both, leading zeros and surrounding whitespace. Key
storage on the displayed form.
