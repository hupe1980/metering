+++
title = "Regulatory basis"
description = "Every provision this library implements, quoted from the published text and dated — including the ones it deliberately does not claim."
weight = 11
+++

Every citation below was checked against the current published text. Where a
claim could not be verified, the library says so rather than asserting it.

| Topic | Basis |
|---|---|
| Ersatzwertbildung, Datenübermittlung | § 60 Abs. 1, 2 MsbG; procedures per BNetzA BK6-24-174, VDE-AR-N 4400 |
| Löschung / Anonymisierung | § 60 Abs. 6 MsbG — a **ceiling** of three years, not a retention duty |
| Zählerstandsgangmessung | § 2 Satz 1 Nr. 27 MsbG; BNetzA BK6-24-174, in force 6 June 2025 |
| Spitzenleistung / Jahreshöchstleistung | § 17 Abs. 2 StromNEV |
| iMSys Pflichteinbaufälle | § 29 Abs. 1 MsbG — > 6 000 kWh/a; § 14a agreement; > 7 kW |
| Rollout-Fahrplan | § 45 Abs. 1 MsbG |
| Zeitvariable Netzentgelte | § 14a EnWG Modul 3 — three levels, mandatory for every NB since 1 April 2025 |
| Gemeinschaftliche Gebäudeversorgung | § 42b EnWG + BDEW Anwendungshilfe Solarpaket 1 (v1.0, 25.01.2024) |
| Energy Sharing | § 42c EnWG — in force 1 June 2026; adjacent Bilanzierungsgebiete from 1 June 2028 |
| Dynamische Tarife | § 41a Abs. 2 EnWG — a duty on large **suppliers**, not a metering mandate |
| Gas m³ → kWh | § 33 MessEG; § 25 Nr. 4 and Nr. 7 MessEV; DVGW G 685, G 260 |
| Gas-SLP (SigLinDe), Allokationstemperatur, Kundenwert | BDEW/VKU/GEODE Leitfaden *Abwicklung von Standardlastprofilen Gas* (KoV) — published in full, quoted verbatim |
| Gastag 06:00–06:00 | GaBi Gas / Art. 3 Nr. 6 VO (EU) 312/2014; temperature averaging per the SLP-Gas Leitfaden |
| MaLo-ID Bildungsvorschrift & Prüfziffer | BDEW Anwendungshilfe *Die neue Marktlokations-Identifikationsnummer* (v1.0, 28.04.2017) |
| MeLo-ID / Zählpunktbezeichnung | VDE-AR-N 4400 / DVGW G 2000 — structure only; there is no check digit |
| Warmwasser-Wärmemenge | HeizkostenV § 9 Abs. 2 |
| Netzverluste | § 22 Abs. 1 EnWG |
| Jahresmehr-/-mindermengen | GPKE Kap. 8.4 (BK6-24-174) |
| Zeitangaben, UTC vs. gesetzliche deutsche Zeit | EDI@Energy Allgemeine Festlegungen v6.1b, Kap. 3 |
| OBIS-Wertegruppen | EDI@Energy Codeliste der OBIS-Kennzahlen und Medien; DLMS/COSEM Blue Book |
| SLP-Typtage, Feiertagskalender | BDEW *Hinweise zu den aktualisierten SLP Strom*, 17.03.2025 |
| Netzqualität | EN 50160 |

## Repealed ordinances

**StromNZV and GasNZV were repealed with effect from the end of
31 December 2025** (Art. 15 Abs. 4 des Gesetzes vom 22.12.2023). Their substance
now lives in BNetzA Festlegungen, and this library cites the Festlegungen.

That matters for two provisions commonly still quoted:

- **§ 12 StromNZV** was *"Standardisierte Lastprofile; Zählerstandsgangmessung"*.
  It is not, and never was, about Spitzenleistung.
- **§ 13 Abs. 3 StromNZV** governed Mehr- und Mindermengen. Those are
  **Jahres**mehr- und -mindermengen, settled annually, now under GPKE Kap. 8.4.

## What is deliberately not claimed

- **The 2025 SLP Dynamisierungsfunktion.** The BDEW Anwendungshilfe publishes it
  as an **image**, so its coefficients cannot be read out of the document,
  quoted or verified. `Dynamization::vdew_1999()` is the 1999 VDEW quartic and
  is documented as exactly that; a `DynamicSlpProfile` carries whichever function
  came with its licensed tables, and refuses to answer without one.
- **G 685 final rounding.** Published Netzbetreiber Merkblätter demonstrably
  diverge between whole-kWh and two-decimal results, and the normative text is
  not freely citable. It is a setting.
- **VDE-AR-N 4400 thresholds.** A paywalled Anwendungsregel whose text cannot be
  reproduced here, so every threshold is a parameter with a documented default.
- **EN 50160 unbalance.** Needs phase angles this data does not carry.

## A note on § 60 Abs. 6 MsbG

It is a **deletion** duty, and is often read as its opposite:

> Der Messstellenbetreiber muss personenbezogene Messwerte […] löschen oder […]
> anonymisieren, sobald […] eine Speicherung […] nicht mehr erforderlich ist,
> spätestens jedoch nach drei Jahren ab dem Schluss des Kalenderjahres […]

Three years is the outer limit, not a requirement to keep. A system built to
retain for three years *because the law says so* has the provision backwards.

Nothing here is legal advice.
