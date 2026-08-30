+++
title = "Regulatory basis"
description = "Every provision this library implements, quoted from the published text and dated — including the ones it deliberately does not claim."
weight = 13
+++

Every citation below is checked against the current published text — most
recently in August 2026, against the EDI@Energy versions binding since
1 April 2026. Where a claim cannot be verified, the library says so rather than
asserting it.

*Allgemeine Festlegungen* v6.1d becomes binding on 1 October 2026 and carries
every passage cited here unchanged, so the v6.1c references below stay accurate
until then.

## Almost nothing here is a measured value

A billing period is a sum, a register delta a difference, a gas kWh a product,
an allocation share a quotient. § 33 Abs. 1 MessEG permits a value for a
Messgröße only if a Messgerät determined it, so every one of those would be
unusable — and **§ 25 Nr. 7 MessEV** is the exception the German energy market
bills on:

> Messgrößen im Bereich der leitungsgebundenen Energieversorgung mit
> Elektrizität und Gas und anderen Energieträgern, deren Werte als Summe,
> Differenz, Produkt oder Quotient oder Kombinationen davon aus Messwerten
> gebildet werden, die mit einem dem Mess- und Eichgesetz und dieser Verordnung
> entsprechendem Messgerät ermittelt worden sind und sofern die Art der
> Berechnung und die verwendeten Werte für den vorgesehenen Verwendungszweck
> geeignet sind

The closing clause is the specification: the *method* and the *values used* must
be fit for the purpose, which means they must be stateable. That is behind most
of the [design constraints](@/docs/design.md):
`GasConversionParams` has no `Default` and invents no Brennwert; a `QualityFlag`
travels with every interval; `substitute` writes an audit trail; a validation
result reports which rules actually ran; a Netzbetreiber's rounding is a
parameter rather than a constant. Every number the library returns can be
re-derived from what it was handed.

§ 25 Nr. 4 is the narrower sibling that permits the Brennwert itself, *"wenn sie
nach den anerkannten Regeln der Technik ermittelt worden sind und die dafür
verwendeten Messwerte mit einem dem Mess- und Eichgesetz und dieser Verordnung
entsprechendem Messgerät ermittelt worden sind"* — DVGW G 685 being the
anerkannte Regel in question.

| Topic | Basis |
|---|---|
| Ersatzwertbildung, Datenübermittlung | § 60 Abs. 1, 2 MsbG; procedures per BNetzA BK6-24-174, VDE-AR-N 4400 |
| Löschung / Anonymisierung | § 60 Abs. 6 MsbG — a **ceiling** of three years, not a retention duty |
| Zählerstandsgangmessung | § 2 Satz 1 Nr. 27 MsbG; BNetzA BK6-24-174, in force 6 June 2025 |
| Spitzenleistung / Jahreshöchstleistung | § 17 Abs. 2 StromNEV |
| iMSys Pflichteinbaufälle | § 29 Abs. 1 MsbG — > 6 000 kWh/a; § 14a agreement; > 7 kW |
| Rollout-Fahrplan | § 45 Abs. 1 MsbG |
| Zeitvariable Netzentgelte | § 14a EnWG Modul 3 (BNetzA BK8-22/010-A) — three levels, mandatory for every NB since 1 April 2025 |
| Modul-3-Rahmenbedingungen | BDEW *Anwendungshilfe für die Umsetzung von Modul 3* v1.1, 07.02.2025, §2 — HT ≥ 2 h/Tag, ganzjährig identische Zeitfenster, ≥ 2 Quartale, nur mit Modul 1, iMSys, kein RLM |
| Netzorientierte Steuerung, netzwirksamer Leistungsbezug | BNetzA **BK6-22-300** Anlage 1 (27.11.2023, in Kraft 01.01.2024) Ziff. 2.3 — a definition, not a formula |
| Mindestleistung `P_min,14a` | BK6-22-300 Anlage 1 Ziff. 4.5.1 / 4.5.2, mit Gleichzeitigkeitsfaktor-Tabelle — quoted verbatim; 0,4 und die GZF sind Vermutungen *"bis zum Inkrafttreten einer anderweitigen Empfehlung"* |
| Unsymmetrieleistung 4,6 kVA | VDE-AR-N 4100 Abschnitt 5.5.2, erläutert im VDE-FNN-Hinweis *Symmetrischer Anschluss und Betrieb in Kundenanlagen* — the Anwendungsregel itself is paywalled, so the limit is a parameter |
| Marktpartner-ID (BDEW-/DVGW-Codenummer) | BDEW *Identifikatoren in der Marktkommunikation* v1.2, §2.2 und §6.1 — Prüfziffer wie MaLo-ID, **außer** bei einer GS1-GLN |
| Gemeinschaftliche Gebäudeversorgung | § 42b EnWG + BDEW Anwendungshilfe Solarpaket 1 (v1.0, 25.01.2024) |
| Energy Sharing | § 42c EnWG — in force **22 December 2025** (BGBl. 2025 I Nr. 347); the Netzbetreiber duty of Abs. 4 from 1 June 2026, adjacent Bilanzierungsgebiete from 1 June 2028 |
| Dynamische Tarife | § 41a Abs. 2 EnWG — a duty on large **suppliers**, not a metering mandate |
| Gas m³ → kWh | § 33 MessEG; § 25 Nr. 4 and Nr. 7 MessEV; DVGW G 685 Teil 2 (Brennwert), Teil 3 (Volumen im Normzustand), Teil 6 (K-Zahl); G 260 |
| Normzustand, Zustandszahl, Höhenzonen | DIN 1343 (`T_n` = 273,15 K, `p_n` = 1013,25 mbar); DVGW G 685-3 — `T_eff` = 15 °C als Festwert, `p_amb = 1016 − 0,12 × H`, Zonenhöhe max. 50 m von der Zonengrenze |
| Gas-SLP (SigLinDe), Allokationstemperatur, Kundenwert | BDEW/VKU/GEODE Leitfaden *Abwicklung von Standardlastprofilen Gas*, KoV XV, Stand 27.03.2026, Anlage 6 — published in full, quoted verbatim |
| Gas-SLP-Codes (15 Typen, inkl. `GHD`) | EDI@Energy *Codeliste TUM- und BDEW-SLP Gas* v1.1, §6.1–6.3 |
| Gastag 06:00–06:00 | GaBi Gas / Art. 3 Nr. 6 VO (EU) 312/2014; temperature averaging per the SLP-Gas Leitfaden |
| MaLo-ID Bildungsvorschrift & Prüfziffer | BDEW Anwendungshilfe *Die neue Marktlokations-Identifikationsnummer* (v1.0, 28.04.2017) |
| MeLo-ID / Zählpunktbezeichnung | VDE-AR-N 4400 / DVGW G 2000 — structure only; there is no check digit |
| Warmwasser-Wärmemenge | HeizkostenV § 9 Abs. 2 |
| Netzverluste | § 22 Abs. 1 EnWG |
| Jahresmehr-/-mindermengen | GPKE Kap. 8.4 (BK6-24-174) |
| Zeitangaben, UTC vs. gesetzliche deutsche Zeit | EDI@Energy Allgemeine Festlegungen v6.1c, Kap. 3 (verbindlich ab 01.04.2026) |
| OBIS-Wertegruppen | EDI@Energy *Codeliste der OBIS-Kennzahlen und Medien* v2.5c (verbindlich ab 01.04.2026), §2.1–2.3; DLMS/COSEM Blue Book |
| Vorzeichen von Mengenangaben | Codeliste v2.5c §2.1 — positiv oder 0, Ausnahme Korrekturenergiemengen |
| Bilanzierungsmonat Strom 00:00, Gas 06:00 | EDI@Energy Allgemeine Festlegungen v6.1c, Kap. 3.1 |
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
- **EN 50160 voltage unbalance.** Needs phase angles this data does not carry.
  Load unbalance is a different quantity and *is* computed — see
  [power quality](@/docs/power-quality.md).
- **How the netzwirksamer Leistungsbezug is apportioned.** BK6-22-300 Anlage 1
  Ziff. 2.3 *defines* the share; it does not say how to split a grid draw that
  local generation partly covered. VDE FNN's *Bewertung der Mindestleistung*
  (V1.0, April 2025) says the calculation is not its subject and points on to
  *Netzbetrieb mit Flexibilitäten* Kap. 4.1.2, which is not freely citable. So
  the convention is the caller's, named and documented, not assumed.
- **The BDEW-Codenummer check digit, as a gate.** The Bildungsvorschrift names
  the Lok- und Waggon-Verfahren and then exempts GS1-issued GLNs, so a valid
  Marktpartner-ID may fail it. `BdewCode` reports the outcome instead of
  refusing the value.
- **MiSpeL — Marktintegration von Speichern und Ladepunkten.** The
  Abgrenzungs- and Pauschaloption apportion storage flows so that
  Umlageprivilegien (§ 21 EnFG) and Marktprämien (§ 19 EEG) can be computed on
  them. The quantities are defined by the payments they size, the Arbeitsstand
  of 05.08.2026 still writes its Bekanntgabe as `[01.10.2026]` in square
  brackets, and part of it waits on EU state-aid approval. Not a rule yet, and
  not this crate's kind of quantity.
- **Leap seconds.** Allgemeine Festlegungen Kap. 3.9 permits a second-precision
  timestamp to name one (`23:59:60`); `time::OffsetDateTime` has no such second,
  so such a value fails at the parse rather than silently becoming `23:59:59`.
  Interval boundaries are quarter-hours and never land there.

## A note on § 60 Abs. 6 MsbG

It is a **deletion** duty, and is often read as its opposite:

> Der Messstellenbetreiber muss personenbezogene Messwerte […] löschen oder […]
> anonymisieren, sobald […] eine Speicherung […] nicht mehr erforderlich ist,
> spätestens jedoch nach drei Jahren ab dem Schluss des Kalenderjahres […]

Three years is the outer limit, not a requirement to keep. A system built to
retain for three years *because the law says so* has the provision backwards.

Nothing here is legal advice.
