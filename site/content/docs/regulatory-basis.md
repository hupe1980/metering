+++
title = "Regulatory basis"
description = "Every provision this library implements, cited from the published text and dated — and the ones it deliberately does not claim."
weight = 15
[extra]
group = "Reference"
+++

Every provision is cited from the published text with its version and date.
Every German passage presented as verbatim source text — in the code, this
site and the README — is matched against the published PDF by `just quotes`;
the one recorded exception is a label inside a figure.

## In force

Re-verified October 2026.

| Source | Version | Binding | Until |
|---|---|---|---|
| EDI@Energy *Allgemeine Festlegungen* | **6.1d** | 01.10.2026 | — |
| EDI@Energy MSCONS | **MIG 2.5** / AHB 3.2 | 01.10.2026 | — |
| EDI@Energy UTILTS | AHB 1.1, MIG 1.1e | 01.10.2026 | — |
| EDI@Energy *Codeliste der OBIS-Kennzahlen und Medien* | **2.5c** | 01.04.2026 | — |
| EDI@Energy *Codeliste TUM-/BDEW-SLP Gas* | 1.1 | — | — |
| SLP-Gas Leitfaden (BDEW/VKU/GEODE) | KoV **XV**, Stand 27.03.2026 | 01.10.2026 | — |
| BNetzA **MiSpeL** (618-25-02) | adopted 01.10.2026 | Pauschaloption: from the month after the EU state-aid approval of § 19 Abs. 3c EEG | — |
| BNetzA BK6-23-241, BilAReM | Beschluss 07.05.2026 | Anlage: 01.10.2026 | Pauschal-Abrechnung: 31.12.2028 |
| StromNEV § 17, Anlage 4 | consolidated | — | **31.12.2028** |
| StromNZV, GasNZV | — | — | **repealed** with the end of 31.12.2025 |

## Derived values: § 25 MessEV {#almost-nothing-here-is-a-measured-value}

A billing period is a sum, a register delta a difference, a gas kWh a product,
an allocation share a quotient. § 33 Abs. 1 MessEG admits a value for a
Messgröße only if a Messgerät determined it; **§ 25 Nr. 7 MessEV** is the
exception the market bills on:

> Messgrößen im Bereich der leitungsgebundenen Energieversorgung mit
> Elektrizität und Gas und anderen Energieträgern, deren Werte als Summe,
> Differenz, Produkt oder Quotient oder Kombinationen davon aus Messwerten
> gebildet werden, die mit einem dem Mess- und Eichgesetz und dieser Verordnung
> entsprechendem Messgerät ermittelt worden sind und sofern die Art der
> Berechnung und die verwendeten Werte für den vorgesehenen Verwendungszweck
> geeignet sind

The method and the values used must be fit for purpose, so they must be
stateable: no default Brennwert, a `QualityFlag` on every interval, an audit
trail on every substitute, the rules that ran on every report, a
Netzbetreiber's rounding as a parameter. § 25 Nr. 4 admits the Brennwert
itself, *"wenn sie nach den anerkannten Regeln der Technik ermittelt worden
sind und die dafür verwendeten Messwerte mit einem dem Mess- und Eichgesetz und
dieser Verordnung entsprechendem Messgerät ermittelt worden sind"* — DVGW G 685.

## Citations

| Topic | Basis |
|---|---|
| Ersatzwertbildung, Datenübermittlung | § 60 Abs. 1, 2 MsbG; procedures per BNetzA BK6-24-174, VDE-AR-N 4400. The Smart-Meter-Gateway placement of Abs. 2 is **conditional** on a BSI assessment and a BNetzA Festlegung; until then Satz 2 permits the preparation outside the gateway |
| Löschung / Anonymisierung | § 60 Abs. 6 MsbG — a **deletion** duty with a ceiling of three years, not a retention duty, owed *"unter Beachtung mess- und eichrechtlicher Vorgaben"* |
| Zählerstandsgangmessung | § 2 Satz 1 Nr. 27 MsbG; BNetzA BK6-24-174, in force 6 June 2025 |
| Spitzenleistung / Jahreshöchstleistung | § 17 Abs. 2 StromNEV |
| iMSys Pflichteinbaufälle | § 29 Abs. 1 MsbG — > 6 000 kWh/a (Nr. 1); § 14a agreement (Nr. 2a); a plant > 7 kW (Nr. 2b). The grounds are **cumulative**, the Steuerungseinrichtung belongs to **Nr. 2 as a whole**, and Nr. 2b applies only *"soweit dies erforderlich ist"* to meet the § 45 quotas |
| Steuerungseinrichtung, waiver | § 29 Abs. 5 MsbG — lifted where feed-in is permanently limited to 0 % **and** declared in Textform; both, or neither |
| Moderne Messeinrichtung | § 29 Abs. 3 MsbG — everything the iMSys duty does not reach, by 31 December 2032 (new and majorly renovated buildings: at completion) |
| Rollout-Fahrplan | § 45 Abs. 1 **Nr. 4** MsbG — the ordinary Letztverbraucher track. Nr. 1–3 are three further schedules, two of them measured in installed **kilowatts** rather than in Messstellen |
| Zeitvariable Netzentgelte | § 14a EnWG Modul 3 (BNetzA BK8-22/010-A) — three levels, mandatory for every NB from 1 April 2025 |
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
| Gas-SLP-Codes (inkl. `GHD`) | EDI@Energy *Codeliste TUM- und BDEW-SLP Gas* v1.1, §6.1–6.3 |
| Gastag 06:00–06:00 | GaBi Gas / Art. 3 Nr. 6 VO (EU) 312/2014; temperature averaging per the SLP-Gas Leitfaden |
| MaLo-ID Bildungsvorschrift & Prüfziffer | BDEW Anwendungshilfe *Die neue Marktlokations-Identifikationsnummer* (v1.0, 28.04.2017) |
| MeLo-ID / Zählpunktbezeichnung | VDE-AR-N 4400 / DVGW G 2000 — structure only; there is no check digit |
| Netzverluste; Jahresmehr-/-mindermengen | § 22 Abs. 1 EnWG; GPKE Kap. 8.4 (BK6-24-174) |
| Zeitangaben (UTC vs. gesetzliche deutsche Zeit), Bilanzierungsmonat Strom 00:00, Gas 06:00 | EDI@Energy Allgemeine Festlegungen 6.1d, Kap. 3 and 3.1 |
| OBIS-Wertegruppen, Vorzeichen | EDI@Energy *Codeliste der OBIS-Kennzahlen und Medien* v2.5c, §2.1–2.3 (Mengen positiv oder 0, Ausnahme Korrekturenergiemengen); DLMS/COSEM Blue Book |
| SLP-Typtage, Feiertagskalender | BDEW *Hinweise zu den aktualisierten SLP Strom*, 17.03.2025 |
| Netzqualität | EN 50160 |
| Benutzungsstundenzahl | § 17 Abs. 1 StromNEV; Anlage 4 zu § 17 Abs. 2 — the Gleichzeitigkeitsgrad's two lines meet *"durch die Jahresbenutzungsdauer 2 500 Stunden"* and reach 1 at 8 760 Stunden |
| Blindmehrarbeit | No national rule. The Freigrenze is the Netzbetreiber's, from its Preisblatt — 50 % der Wirkarbeit or `cos φ = 0,9`; both are offered, neither presumed |
| Wertestatus, Ersatzwertbildungsverfahren, Grund | EDI@Energy **MSCONS MIG 2.5**: `QTY` (`220` Wahrer Wert, `67` Ersatzwert, `187` Prognosewert, `Z18` Vorläufiger Wert, `20` Nicht verwendbarer Wert); `STS+Z32` (`Z92` Interpolation, `Z93` Haltewert (Gas), `Z95` Historische Messwerte (Gas), `ZJ2` Statistische Methode (Strom)); `STS+Z40` (`SubstitutionReason`) |
| Plausibilisierung, Ersatzwertverfahren | VDN *MeteringCode 2006* Anlage 7 and A8.2.2; cross-checked against § 55 Abs. 2 ElWG (AT), NL Meetcode § 5.4.3, Elexon BSCP502 § 4.1.5/§ 4.2, CPUC VEE Rev. 2.0 § 4.1 |
| Berechnungsformel | EDI@Energy UTILTS AHB 1.1 / MIG 1.1e (PID 25001); BDEW AWH *Beispiele von Berechnungsformeln für das Solarpaket 1* v1.1 |
| Ausfallarbeit | § 13a Abs. 1a, 2 EnWG; BNetzA BK6-23-241 with BilAReM Kap. 3; BDEW *Leitfaden zur Berechnung der Ausfallarbeit Redispatch 2.0* |
| MiSpeL | BNetzA Festlegung 618-25-02 (01.10.2026), Anlage 1 (Abgrenzungsoption) and Anlage 2 (Pauschaloption); § 21 EnFG, § 19 EEG |
| Negative Preise; Zeitgleichheit | EEG § 51 Abs. 1 and 3, § 51a; EnFG § 46 Abs. 3 and 5 |
| Heizkosten | HeizkostenV § 6a Abs. 2, § 9 Abs. 2 and 3, § 9a |

**StromNZV and GasNZV** were repealed with effect from the end of 31 December
2025 (Art. 15 Abs. 4 des Gesetzes vom 22.12.2023); their substance is in BNetzA
Festlegungen, which this library cites. § 12 StromNZV (*"Standardisierte
Lastprofile; Zählerstandsgangmessung"*) was never about Spitzenleistung; the
Mehr- und Mindermengen of § 13 Abs. 3 are **Jahres**mehr- und -mindermengen
under GPKE Kap. 8.4.

## Deliberately not claimed

| Question | Why it is the caller's | Where |
|---|---|---|
| A Dynamisierungsfunktion beyond the published one | The BDEW AWH SLP Strom 2025 prints its formula as an image; transcribed, it is the 1999 VDEW quartic. A profile without a function answers nothing | `Dynamization::BDEW` |
| G 685 final rounding | Merkblätter diverge between whole kWh and two decimals; the normative text is not freely citable | `G685Rounding` |
| Whether a particular plant > 7 kW is due | § 29 Abs. 1 Nr. 2b depends on the Messstellenbetreiber's portfolio | `RolloutObligation::is_quota_conditional` |
| The Blindarbeit Freigrenze | No national rule; both published ratios are constants, a third is passed in. `tan(arccos(cos φ))` is stated, not computed — a square root has no exact decimal | `ReactiveLimit` |
| VDE-AR-N 4400 thresholds | Paywalled; every threshold is a setter with a documented default | `Rules` |
| EN 50160 voltage unbalance | Needs phase angles three RMS magnitudes do not carry | [Power quality](@/docs/power-quality.md) |
| How the netzwirksamer Leistungsbezug is apportioned | Ziff. 2.3 defines the share, not the split; VDE FNN *Bewertung der Mindestleistung* (V1.0, April 2025) defers to *Netzbetrieb mit Flexibilitäten* Kap. 4.1.2, not freely citable | `Verursachungsregel` |
| The BDEW-Codenummer check digit as a gate | GS1-issued GLNs are exempt, so a valid ID may fail it | `BdewCode::issuer` |
| Whether MiSpeL's Pauschaloption applies | Depends on the EU state-aid approval of § 19 Abs. 3c EEG | `eeg::mispel::pauschal` |

## Watch list

Dated regulatory changes and the modules they touch.

| When | What | Affects |
|---|---|---|
| within 2026 | BNetzA *AgNeS* Festlegung (GBK-25-01-1#3) issues; applies from 01.01.2029 — a booked capacity replaces the Leistungspreis on a measured peak | `billing::aggregation` |
| open | EU state-aid approval of § 19 Abs. 3c EEG | MiSpeL Pauschaloption applicability |
| 31.12.2026 | HeizkostenV § 5 Abs. 3: every device remotely readable | `heat` (§ 6a monthly information) |
| spring 2027, binding 01.10.2027 | KoV XVI and its SLP-Gas Leitfaden | `slp::gas` |
| 30.09.2027 | MiSpeL implementation window ends | `eeg::mispel` |
| 31.12.2027 | GasNEV expires | not cited — no gas Leistungspreis is modelled |
| 01.06.2028 | § 42c sharing into adjacent Bilanzierungsgebiete | `allocation::sharing` |
| 31.12.2028 | StromNEV expires (`STROMNEV_AUSSERKRAFT`); Redispatch Pauschal-Abrechnung ends (`PAUSCHAL_BESTANDSSCHUTZ_ENDE`) | `billing::aggregation`, `billing::reactive`, `grid::ausfallarbeit` |
| 31.12.2032 | § 29 Abs. 3 MsbG moderne Messeinrichtung everywhere (`MME_DEADLINE`) | `grid::rollout` |
| rolling | BNetzA Datenformat-Mitteilungen (latest Nr. 58) and GPKE-Mitteilungen (latest Nr. 73) | EDI@Energy versions above |

Nothing here is legal advice.
