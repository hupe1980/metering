# Changelog

All notable changes to `metering` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the crate follows
semver, with the `serde` representation explicitly in scope (see the crate docs).

## [0.18.0] — 2026-08-15

Two gaps closed and two identifiers typed. The crate validated OBIS codes to
the digit while carrying MaLo-IDs as unchecked strings — its own doc examples
used a MaLo whose check digit is wrong. And it modelled gas conversion, gas
quality configs and a gas balancing method while offering no way to compute
the one thing a gas SLP delivery point is settled on: the temperature-driven
daily profile. Every removal remains a hard cut; no deprecated shims.

### Added

- **`ids`** — [`MaloId`] and [`MeloId`]. `MaloId` enforces the full BDEW
  Bildungsvorschrift at the parse — eleven digits, Vergabestelle in 1–9, and
  the check digit of the Anwendungshilfe's "Lok- und
  Waggon-Kennzeichnungsverfahren", whose worked example (`4137355924` → `1`)
  the tests reproduce digit for digit. The docs state the scheme's one blind
  spot (a ±5 change in an even position) instead of implying Luhn-grade
  protection. `MeloId` validates the 33-character Zählpunktbezeichnung
  structurally and canonicalises to uppercase. Both serialise as their
  canonical strings.
- **`gas_slp`** — the published SigLinDe/TUM gas SLP arithmetic from the
  BDEW/VKU/GEODE Leitfaden *Abwicklung von Standardlastprofilen Gas*:
  `h(ϑ) = A/(1+(B/(ϑ−ϑ₀))^C) + D + max(mH·ϑ+bH, mW·ϑ+bW)`, the
  geometric-series Allokationstemperatur (exact — the weights are eighths),
  `Q(D) = KW·h(ϑ)·F_WT`, Kundenwert derivation, and weekday factors that must
  sum to 7.0000 and give Feiertage the Sunday factor (nationwide by default,
  per-Land on request) — each rule quoted from the Leitfaden. The published
  `DE_HEF34` coefficient row is embedded and its printed normalisation
  `h(8 °C) = 1.00000` is pinned by a test.
- **`calendar::gas_day_start_utc` / `gas_day_end_utc` / `local_gas_day`** —
  the 06:00–06:00 Gastag. Summing a gas Lastgang over the calendar day books
  six hours into the wrong Bilanzierungstag, every day. The 23/25-hour Gastag
  is the one named after the *Saturday* — the clocks change before 06:00 —
  which the tests pin.

### Changed (breaking)

- **The gas `LoadProfile` codes are now the real ones.** `GasEF`/`GasMF`/
  `GasGHD` (codes `EF`, `MF`, `GHD`) were not BDEW gas profile codes; `GHD`
  does not exist at all. The gas variants are now the fourteen published
  TUM/FfE types — `HEF`, `HMF`, `HKO` and the eleven Gewerbe profiles `GKO`,
  `GHA`, `GMK`, `GBD`, `GGA`, `GBH`, `GWA`, `GGB`, `GBA`, `GPD`, `GMF`.
  `parse` accepts `EF`/`MF` as lenient aliases; `GHD` is a hard error because
  there is no profile it could honestly map onto.
- **`LoadProfile` serde tags now equal the `as_str` codes** for every variant.
  The derived representation wrote the Rust variant names, so `GasEF`
  serialised as `"GasEF"` (not `"EF"`) and `Custom` as `"Custom"` (not
  `"CUSTOM"`) — two spellings per value, in violation of the crate's stated
  one-string rule. `tests/serde_representation.rs` now pins every tag.
- **`MeasurementPoint`, `MeasurementSeries` and the lifecycle events carry
  `MaloId` / `MeloId`** instead of `String`. `MeasurementSeries::new` takes a
  `MaloId`. Virtual-meter `SourceMap` keys deliberately stay `String` — they
  are arbitrary series labels, not asserted MaLo-IDs.
- **`MeasurementSource::is_billable_source` is removed.** Billability is a
  property of each interval's `QualityFlag` — only `Faulty` and `Unknown`
  block billing — never of provenance. The predicate said a manual entry and
  a GGV virtual-meter result were unbillable; both are billed every day. A
  second, contradictory notion of billability is worse than none.
- **`MarktRolle::is_mscons_receiver` now includes `Uenb`.** Under MaBiS the
  Netzbetreiber transmits the Bilanzierungs-Summenzeitreihen to the ÜNB as
  Bilanzkoordinator via MSCONS; reporting the ÜNB as a non-receiver was
  simply wrong.

### Fixed

- **`fill_gaps` Vergleichstag window vs. the autumn fall-back.** The
  `PriorPeriodAverage` reference window was a fixed `Duration::days(7)` —
  168 hours — while slots are matched on Berlin (weekday, hour, minute),
  which recurs every 7 *local* days. Across the October transition the
  matching slot is 169 UTC hours back, fell outside the window, and the
  configured method silently degraded to `LastValueCarryForward` for the week
  after every fall-back. The window is now seven Berlin calendar days
  (`calendar::shift_back_days`, new — the day-granular sibling of
  `shift_back_one_year`).
- **`fill_gaps` interpolation geometry across faulty slots.** The gap length
  was measured to the next *present* slot (quality-blind) while the closing
  value came from the next *billable* one — so with a present-but-faulty slot
  bordering a gap, the line reached the closing value at the faulty slot's
  position and every interior value sat at the wrong fraction. The mirror
  defect existed on the preceding side. Interpolation now anchors on the
  billable values either side at their **true grid-slot distances**, so all
  missing slots between two billable anchors land on one straight line
  however faulty slots partition them. Faulty slots themselves are still
  passed through untouched, never substituted.
- The sample MaLo-ID used across the documentation (`51238696780`) failed the
  BDEW check digit. It is now `51238696781`, which passes — and, since the
  fields are typed, could not have survived otherwise.

## [0.17.0] — 2026-08-10

A correctness release, and a large one. Several statutory citations were wrong —
one of them inverted the obligation it described. A validation rule could not
fire on the data model it validated, another diagnosed the wrong fault. Three
modules computed nothing or duplicated a fourth. Substitute-value interpolation
was off by one interval and the forecast interval was about five times too
narrow.

Every removal is a hard cut; there are no deprecated shims.

### Removed

- **`smgw`** — 570 lines of Smart-Meter-Gateway records: statuses, certificate
  metadata, CLS channels. Its entire computational content was two date
  subtractions, and nothing else in the crate referenced any of it. Certificate
  lifecycle is a PKI problem, and a module whose own docs say it does not parse
  X.509 has no business modelling it. The one metering-side concept — a gateway
  outage requiring an Ersatzwert — already exists as
  `SubstitutionReason::GatewayCommFailure`.
- **`tariff_window`** — `TariffWindow`, `TariffWindowDays` and `HtNtSchedule`.
  See *one tariff-register mechanism* below.
- **`register`** — merged into `measurement_point`. See *one measurement point*
  below.
- **`demand`** — after `DemandWindow` went, what remained was a `DemandInterval`
  nothing constructed and an `energy_to_demand_kw()` duplicating
  `MeterInterval::demand_kw`.
- **`validation` V10 (register rollover)** and its rule number, retired rather
  than recycled. It compared consecutive `value_kwh` for a drop over 50 000 kWh,
  but a `MeterInterval` carries interval energy, not a cumulative Zählerstand:
  for it to fire, one quarter-hour had to carry 50 MWh — 200 MW of average load.
  Rollover detection now lives in `reading`, where readings are. Leaving `V10`
  unused means a stored finding cannot be silently reinterpreted.
- **`quality::score_intervals_raw` and `score_intervals_f64`** — two more copies
  of gap detection, zero-run counting, interval consistency and coverage, each
  subtly different from the validation engine's. A series could grade `A` while
  validation reported errors on it. The `score_intervals_f64` docs also claimed
  SIMD auto-vectorisation of loops that allocate a `Vec` per window.
- **`forecast::substitute_values`, `ForecastMethod`, `SubstituteValueEntry`** —
  a second Ersatzwertbildung engine. See *one substitute engine* below.
- **`substitute::linear_interpolation`** — its time fraction was `mid/total`,
  which is always `0.5`: a midpoint average dressed up as an interpolation.
- **`AggregationConfig::include_ht_nt` / `ht_window` / `rlm_zweitarif`,
  `BillingPeriod::ht_nt`, `HtNtSplit`** — see *one tariff-register mechanism*.
- **`impl Default for PowerQualityInterval`** — it invented a zero-length
  interval at the Unix epoch. Use `PowerQualityInterval::empty(from, to)`.
- **`MeterRegister::register_number`** — duplicated OBIS value group E, was free
  to contradict it, and documented itself as 0–9 when E has run 0–62 since
  2023-10-01. Use `ObisCode::tariff_register()`.

### Fixed — statutory citations

These were wrong, not merely imprecise. Each module now states the correction in
its docs rather than quietly changing it.

- **§ 60 Abs. 6 MsbG was described backwards.** It was cited as *"3-year
  retention with full provenance for billing data"*. It is a **deletion** duty:
  personenbezogene Messwerte must be erased or anonymised as soon as they are no
  longer needed, *"spätestens jedoch nach drei Jahren ab dem Schluss des
  Kalenderjahres"*. Three years is a ceiling. A system built to retain for three
  years *because the law says so* has it inverted.
- **§ 60 Abs. 2 MsbG was over-read.** It names Plausibilisierung and
  Ersatzwertbildung and says they should happen in the Smart-Meter-Gateway. It
  prescribes no method, reference period or ranking — so claims such as
  *"prior-period same-slot average is the preferred method per § 60 Abs. 2"* are
  gone. The duty is § 60 Abs. 1; the procedures are BNetzA Festlegungen
  (BK6-24-174) and VDE-AR-N 4400.
- **`substitute` carried a self-contradictory sentence**: *"the MsbG was
  repealed by Art. 12 G. v. 29.08.2016 and folded into the MsbG"*.
- **Spitzenleistung was cited as § 12 StromNZV and § 18 Abs. 1 StromNEV.**
  Neither is about peak demand: § 12 StromNZV was *"Standardisierte Lastprofile;
  Zählerstandsgangmessung"* (repealed with effect from the end of 31.12.2025),
  and § 18 StromNEV is *"Entgelt für dezentrale Einspeisung"*. It is
  **§ 17 Abs. 2 StromNEV**.
- **BK6-22-024 was cited for GPKE, MaBiS and MMM billing.** It is the
  *Lieferantenwechsel in 24 Stunden* (LFW24) Festlegung. Replaced with
  BK6-06-009 (GPKE), BK6-07-002 (MaBiS) and the consolidated Lesefassung
  **BK6-24-174**.
- **§ 42a EEG was cited for residual-load metering.** No such provision exists;
  § 42a EnWG is *Mieterstromverträge*. Residual load is arithmetic the market
  does not legislate, so it now carries no citation.
- **§ 42b Abs. 5 EnWG was misquoted.** The quoted per-tenant sentence does not
  appear in it. Abs. 5 caps the **pool** at the lesser of generation and total
  participant consumption; the per-tenant `max(0, …)` is the BDEW
  Anwendungshilfe's `Pos()` operator. Both are now stated, separately.
- **§ 29 MsbG appeared with three different meanings** across `rollout`, `smgw`
  and `power_quality`. It is *"Ausstattung von Messstellen mit intelligenten
  Messsystemen …"*, and only `rollout` implements it.
- **§ 41a Abs. 2 EnWG was read as a 15-minute resolution mandate.** It obliges
  suppliers with more than 100 000 customers to *offer* a dynamic tariff to
  customers who have an iMSys.
- **Mehr-/Mindermengen were described as monthly.** They are **Jahres**mehr- und
  -mindermengen (GPKE Kap. 8.4).
- **`calendar` cited § 20 StromNZV** for German local time. Replaced with the
  EDI@Energy *Allgemeine Festlegungen* v6.1b, Kap. 3, which states the UTC /
  gesetzliche deutsche Zeit split directly.
- **`register` documented OBIS group D as the direction** (`8` = import,
  `9` = export), contradicting `obis`, which correctly places direction in
  group C. `9` is Vorschub.
- **`lifecycle` called the MeLo-ID 11 digits.** It is 33 characters; the MaLo-ID
  is 11 digits.
- **The 2025 SLP Dynamisierungsfunktion is no longer assumed.** The lookup
  applied the 1999 VDEW quartic to H25, P25 and S25 on the assumption that the
  revision retained it. The BDEW Anwendungshilfe publishes its function as an
  **image**, so the coefficients cannot be read out, quoted or compared.
  `Dynamization::vdew()` → **`vdew_1999()`**, documented as the 1999 function and
  only that; `DynamicSlpProfile` gains a `dynamization` field and `value_at`
  returns `None` for a profile that needs one and has none.

### Fixed — behaviour

- **V02 missed overlaps.** It compared each interval only against its immediate
  predecessor, so a long interval swallowing several short ones reported the
  first collision and passed the rest. It now tracks the furthest end seen.
- **V04 could not see a run of spikes.** It compared each value against the
  **mean of the whole series**, which the spikes inflate — so a run of bad
  values raised its own threshold and hid itself, and a global mean judged quiet
  hours against a threshold set by busy ones. Now a Hampel identifier (local
  median ± `t`·MAD·1.4826), whose 50 % breakdown point a spike cannot move.
- **V06 flagged the two correct days of the year.** `expected_interval_secs` is
  a fixed count and a German calendar day is 82 800 s in spring and 90 000 s in
  autumn, so a daily gas or water series drew a warning on both DST days for
  being exactly right.
- **V07 diagnosed the wrong fault.** It compared the whole day's covered
  duration against 25 hours, so *any* two missing quarter-hours on a fall-back
  day produced a confident "the repeated hour 02:00–03:00 was collapsed". It now
  looks only at the two-hour UTC window the two passes occupy, so a midday gap
  is a V01 gap and nothing else.
- **Substitute interpolation was off by one interval.** `forecast` placed the
  substitutes at fractions `0/n … (n-1)/n`, so the first synthesised value
  equalled the last *measured* one and the series never approached the closing
  value — a systematic bias on any rising or falling gap. The fractions are now
  interior: `1/(n+1) … n/(n+1)`.
- **The forecast prediction interval omitted the estimation error.** It computed
  `1.96·σ·√Y`, the spread of a realised year around a *known* daily mean. The
  mean is estimated from `n` observed days, so the variance is `Y²σ²/n + Yσ²`.
  At `Y = 365, n = 14` the missing term is twenty-six times the one present.
- **A long gap silently changed method past 100 intervals.** The gap-length scan
  was capped at 100, and the length was re-measured from a moving cursor, so the
  last `short_gap_threshold` intervals of every long gap reverted to
  interpolation. The length is now measured once, at the gap's first slot.
- **A daily gap fill drifted across DST.** Stepping a fixed 86 400 s puts every
  slot after the last Sunday in March an hour off its Liefertag, so measured
  values stop matching the grid and the remainder of the year is silently
  substituted. The grid is an `IntervalResolution` and walks calendar periods.
- **`normalize_interval_to_kwh` silently mis-scaled unknown units.** It fell
  through to *"assume already kWh"*, so `"MWh"` — which the crate's own
  `MeasurementUnit::parse_scaled` reads correctly — passed unconverted,
  understating the reading a thousandfold. Replaced by `normalize_to_kwh`,
  returning `Result<_, ConversionError>`.
- **`is_bezug` and `is_einspeisung` could both be true**, on both
  `MeasurementPoint` and `MeterRegister`: they combined the master-data
  direction and the OBIS code with `||`.
- **`MeterRegister::is_active` ignored `valid_from`**, so a configuration
  entered in advance took effect the moment it was recorded.
- **`MeterExchangeEvent` clamped backwards readings to zero** under the name
  *"rollover protection"*. It was concealment: a Jahresabrechnung whose old
  register had wrapped billed **0 kWh** for the whole pre-exchange span. The
  three consumption methods now return `Result` and delegate to
  `reading::consumption_between`, which reconstructs the wrap.
- **`ResampledBucket` reported unknown as perfect.** A calendar source
  resolution has no fixed count, which was stored as `0` — making
  `coverage_pct()` return 100 % and `is_complete()` return `true` for a bucket
  nobody could assess.
- **`aggregate` counted non-billable intervals** in `interval_count` while
  excluding them from the sum, and measured coverage against the extent of the
  data itself, so a month whose last week never arrived reported 100 %.
- **`virtual_meter` was quadratic.** Each aligned timestamp did a linear `find`
  per source series — 35 040² probes per source on a year of quarter-hours.
- **`power_quality` claimed EN 50160 conformance from one interval.** Every
  EN 50160 limit is a **share of 10-minute means over an observation window**;
  a week is 1 008 samples and up to 50 may sit outside `Un ± 10 %` with the
  supply conforming. The per-interval predicates are documented as triage
  indicators, and `assess_en50160` answers the standard's actual question.
- **docs.rs would have published the crate without any `serde` impls.** There
  was no `[package.metadata.docs.rs]`, so the hosted documentation would have
  built with default features — hiding the wire representation this crate
  documents as semver-covered.
- Two `as u32` casts could silently truncate a `usize`; a `usize` subtraction in
  V05 was guarded only by an invariant three lines away.

### Changed — one mechanism where there were two

- **One tariff-register mechanism.** `Zaehlzeitdefinition` now covers every
  time-of-use split, with `ht_nt()` for the classic Zweitarif and **`modul_3()`**
  for § 14a EnWG Modul 3. The removed `TariffWindow` could not do the job:
  **Modul 3 has three tariff levels and every Netzbetreiber has been obliged to
  offer it since 1 April 2025**, while `TariffWindow` had two outcomes and
  hour-granularity bounds. Two mechanisms for one question also meant two places
  to fix when Feiertage turned out to matter, and only one got fixed.

  `aggregate` no longer computes the split: it returns one Arbeitsmenge, and the
  breakdown is `Zaehlzeitdefinition::split_energy`, which handles any number of
  registers. The split always reconstructs the total — asserted across both DST
  transitions, where a 00:00–06:00 band holds 20, 24 or 28 quarter-hours.
- **One measurement point.** `MeterRegister` and `MeasurementPoint` modelled the
  same thing with **two different direction enums** that could disagree about
  one register. `EnergyDirection` is gone; `EnergyFlow` is the one direction
  enum and gains `draws_from_grid()`, `feeds_grid()` and `is_storage()`.
  `wandler_factor` and `apply_wandler()` survive on `MeasurementPoint`.
- **One substitute engine.** `forecast`'s Ersatzwertbildung is gone;
  `substitute::SubstituteEntry` carries the audit record, and its `method` field
  reports **what actually ran**, not what was requested.
- **One quality ranking.** `QualityFlag::severity_rank`, `worse_of` and
  `worst_of` replace three private copies in `resample`, `virtual_meter` and
  `measurement_series` that were free to drift apart.
- **One grading path.** `score_intervals` runs the validation engine and grades
  its output, so the two cannot disagree.
- **`RegisterUnit` moved to `obis` and is derived, not stored.**
  `ObisCode::register_unit()` reads it off the value groups: `1-0:3.8.0` is
  kvarh and `1-0:1.6.0` is kW whatever a field says.

### Changed — API

- **`MeterInterval::value_kwh` → `value`.** `Sparte::Wasser` is supported, water
  is billed in cubic metres, and a field named `_kwh` holding m³ is a lie the
  compiler cannot catch. Everything derived follows:
  `BillingPeriod::arbeitsmenge`, `HtNtSplit::{ht, nt}`,
  `ResampledBucket::total`, `MeasurementSeries::total()`, the `AnnualForecast`
  quantities, `MeterLifecycleEvent::reading`, `MeterExchangeEvent`'s readings,
  `ValidationIssue::affected_value`. `demand_kw`, `spitzenleistung_kw` and
  `peak_kw` **keep** their suffix — a power is only meaningful where the unit is
  energy. **This changes the `serde` wire format.**
- **`fill_gaps`** takes `(&[MeterInterval], &FillGapsConfig)` like every other
  entry point. The resolution and period are constructor arguments —
  `FillGapsConfig::new(resolution, from, to)` — because they are the two things
  a gap fill cannot proceed without and the two most easily got wrong.
  `fill_gaps_with_config` is gone; `fill_gaps` returns the `FilledSeries`.
- **`project_annual_consumption`** no longer takes a `malo_id` it never read,
  and `AnnualForecast` no longer carries one.
- **`classify_messtyp`** takes `Option<SeriesOrigin>` instead of `Option<&str>`.
  The old signature matched substrings, so `"LEGACY_NON_SMGW_IMPORT"` classified
  as iMSys and `"Gateway"` did not.
- **`sharing`** takes `Zaehlertyp` and `Bilanzierungsmethode` enums instead of
  `Option<String>` compared against literals, and returns `Vec<Finding>` instead
  of `Vec<String>` of German prose. A record whose source spelled a value
  differently was silently `Disqualified` — not `Unknown` — so a vocabulary
  mismatch looked like a finding rather than a bug.
- **`AggregationConfig`** presets collapse: `rlm_strom()` → **`rlm()`**;
  `slp_strom()` and `gas()`, which had become byte-identical, →
  **`arbeitsmenge_only()`**.
- **`MeasurementSource::VirtualMeter`** carries `rule: VirtualMeterKind` and
  `source_ids` instead of `rule_type: String`; `AggregationRule::rule_type()` →
  `kind()`.
- **`ValidationConfig`**: `spike_factor` → `outlier_sigma` / `outlier_window` /
  `outlier_min_sigma`; `rollover_threshold_kwh` removed; new `period` /
  `over_period()` extends V01 to the head and tail of the series. New rule
  `V12 ImplausiblePower`, split out of `ImpossibleSpike` (now
  `StatisticalOutlier`) so the two can be filtered apart.
- **`QualityConfig`** is `{ validation, max_zero_run_allowed, min_coverage_pct }`;
  `QualityReport` replaces its `Vec<String>` fields with counts plus typed
  `issues`. `QualityGrade` is now `Ord`, best-first.
- **`virtual_meter::SourceMap<S>`** — the source map is generic over its hasher,
  so a caller holding an `FxHashMap` need not rebuild it.
- **`TariffWindowDays`** → `zaehlzeit::DayGroup` (`WeekdaysOnly` → `Weekdays`).
- **`QualityFlag::is_billable` / `is_provisional`** take `self` and are `const`.

### Added

- **`reading`** — Zählerstände and the **Zählerstandsgang → Lastgang**
  conversion, the operation BNetzA **BK6-24-174** (*"Datenübermittlung ZSG"*,
  in force 6 June 2025) makes the Messstellenbetreiber's job. § 2 Satz 1 Nr. 27
  MsbG defines the input; the crate modelled only the output.

  `MeterReading`, `to_lastgang`, `consumption_between`, `Rollover`, `Anomaly`.
  This is where register rollover belongs. Two safeguards keep reconstruction
  from becoming a hazard: without a configured register width nothing is
  reconstructed, and a plausibility cap distinguishes a genuine wrap from an
  undocumented meter exchange. Where no honest difference exists the conversion
  emits **no interval**, so the hole becomes an ordinary V01 gap that
  `substitute` fills with an audit trail.
- **`holiday`** — a computed German statutory holiday calendar: `Bundesland`
  (all sixteen, ISO 3166-2:DE), `Holiday` (nineteen feasts with their Land
  scope), `easter_sunday`, `slp_day_type`. The crate defined `SlpDayType` and
  classified HT/NT by weekday but had no way to produce a day type or to put a
  Feiertag on the off-peak register. Easter is pinned against published dates
  from 2020 to 2285 and asserted to be a Sunday in 22 March – 25 April for 300
  consecutive years.

  It is deliberately **not** a Fristenkalender: counting Werktage to a GPKE
  deadline belongs in a process engine.
- **`power_quality::assess_en50160`** with `En50160Limits`, `En50160Report`,
  `LimitOutcome` and `voltage_percentile` — the statistical test the standard
  actually specifies. Unbalance is not assessed: it needs phase angles that
  three RMS magnitudes do not carry.
- **`calendar::dst_transition_utc(day)`** — the instant the UTC offset changes,
  and the anchor for the repeated hour.
- **`examples/pipeline.rs`** — one Liefertag end to end, on the 25-hour autumn
  DST day with a wrapped register and a corrupt reading planted in it. It
  asserts its own invariants, so CI runs it as a test.
- `ConversionError`, `FilledSeries`, `SubstituteEntry`, `VirtualMeterKind`,
  `ValidationRuleId::code`/`ALL`, `SubstituteMethod::ALL`/`description`,
  `SubstitutionReason::ALL`/`description`, `QualityGrade::ALL`,
  `MIN_OBSERVATION_DAYS`, `AnnualForecast::daily_average`,
  `ValidationResult::by_rule`, `Zaehlzeitdefinition::{registers, until, in_land}`,
  `ZaehlzeitFenster::{new, spanning, on_days, in_months}`,
  `DynamicSlpProfile::value_on`.
- Re-exports that were missing: `gas_m3_to_kwh_hs_rounded`, `G685Rounding`,
  `G685FinalRounding`, `Dynamization`, `DynamicSlpProfile`, `SlpDayType`,
  `Zaehlzeitdefinition`, `ZaehlzeitFenster`, `RolloutObligation`,
  `classify_rollout_obligation`, `Capability`, `Delivery`, `SharingReadiness`,
  `K_MAD`.
- `sharing`'s `Capability`, `Delivery` and `SharingReadiness` now derive
  `Deserialize` as well as `Serialize`; they could be written but not read back.

### Documentation

- **A documentation site** at `site/`, built with Zola and deployed to GitHub
  Pages. Twelve guides covering the calendar, readings, validation,
  Ersatzwertbildung, tariff registers, gas conversion, power quality, virtual
  meters, the full pipeline, the regulatory basis and the design constraints.
  `zola check` runs in CI, so a broken internal link fails the build.
- **The README is a README again** — from 44 KB to about 9 KB. Pitch,
  installation, one worked example, scope and pointers; the depth is on the
  site.
- `tests/readme_samples.rs` → **`tests/doc_samples.rs`**: every code block in
  the README *and* on the site is an executable assertion.
- The crate-level module table listed 18 of 27 modules and described
  `aggregation` as computing an HT/NT split it no longer computes. The opening
  line cited **GasGVV**, which never governed the gas conversion — MessEG and
  MessEV do.

## [0.16.0] — 2026-07-27

One theme: **a value must have exactly one string.** Two spellings of one value
is not a cosmetic problem — it produces two database keys, two map entries and
two "distinct" rows that mean the same thing, with no error raised anywhere.
Reported from production, where a correction failed to supersede the reading it
corrected and the billed total was overstated by the correction amount.

A second theme emerged while verifying the first against the authoritative
source — the EDI@Energy *Codeliste der OBIS-Kennzahlen und Medien* v2.4b.
Several `ObisCode` predicates were written from the IEC value-group *names*
rather than from how the German market *assigns* them, and the test suite
never exercised the difference.

### Fixed — OBIS value-group semantics

- **Direction was read from the wrong value group, so the Lastgang was neither
  import nor export.** `is_import()` required `C = 1 && D = 8`, but EDI@Energy
  §2.1 defines direction by C alone — "+ Bezug des Kunden aus dem Netz (z. B.
  `1-b:1.x.y`)", with `x`/`y` explicitly free. `1-0:1.29.0` — the Lastgang,
  the code MSCONS PID 13018 carries and the one a `MeterInterval` normally
  holds — therefore reported `is_import() == false`, and so did
  `MeterInterval::is_import_energy()`, `MeasurementPoint::is_bezug()` and
  `MeterRegister::is_import()`. Direction is now `A = 1 && C ∈ {1, 2}`, across
  every Messart.

- **`D = 9` was modelled as a reverse-direction flag.** In the German market
  D is a *Messart*: 6 = Maximum, 8 = Zählerstand (Zeitintegral 1), 9 = Vorschub
  (Zeitintegral 2), 29 = Lastgang (Zeitintegral 5). The constants
  `STROM_REACTIVE_INDUCTIVE_EXPORT` / `..._CAPACITIVE_EXPORT` (`1-0:3.9.0`,
  `1-0:4.9.0`) were labelled "export direction" but denote *Blindarbeit
  positiv/negativ, Vorschub* — a different quantity, not a direction.

- **`is_demand()` claimed `D = 29` is maximum demand.** D = 29 is the Lastgang,
  an energy quantity in kWh per interval; the maximum is D = 6, a power in kW.
  Conflating them bills a 15-minute energy quantity as a Leistungspreis basis.
  Replaced by `is_lastgang()`, `is_maximum()`, `is_zaehlerstand()` and
  `is_vorschub()`, which name what each Messart is.

- **`is_reactive()` missed the four quadrant registers and claimed gas volume.**
  It tested `C ∈ {3, 4}` with no medium guard. Blindleistung is C = 3…8 —
  positiv, negativ and Q I…Q IV — so a quadrant register was read as active
  energy, putting kvarh into a kWh column. Without the medium guard,
  `GAS_VOLUME_M3` (`7-0:3.0.0`, C = 3) reported `is_reactive() == true`.
  Now `A = 1 && C ∈ 3..=8`.

- **`tariff_register()` reported the Fehlerregister as tariff 63.** EDI@Energy
  §2.2 lists "63 Fehlerregister" alongside the tariffs; it counts faults and is
  not a billable quantity. It now returns `None`, with `is_fehlerregister()` to
  distinguish it from the total register, and `default_resolution()` returns
  `None` for it rather than a 15-minute series.

- **Two power-quality OBIS codes were documented as L1 when they are the
  all-phase average.** Per-phase channels are 3x (L1), 5x (L2), 7x (L3); the 1x
  codes are the average across phases. `power_quality.rs` listed `1-0:12.7.0`
  as "Voltage L1" (it is the average — a different number on an unbalanced
  three-phase load) and `1-0:11.7.0` as "Current L1". The correct codes are
  `1-0:32.7.0` and `1-0:31.7.0`; the table now lists per-phase and average rows
  separately, and the missing L2/L3 current rows were added.

### Changed — OBIS constants renamed to what they denote

| Removed | Replacement | Why |
|---|---|---|
| `STROM_DEMAND_INTERVAL` | `STROM_BEZUG_LASTGANG` | `1-0:1.29.0` is a Lastgang in kWh, not a demand in kW |
| — | `STROM_BEZUG_MAXIMUM` | `1-0:1.6.0`, the actual Spitzenleistung register |
| — | `STROM_BEZUG_VORSCHUB` | `1-0:1.9.0`, Zeitintegral 2 |
| — | `STROM_EINSPEISUNG_LASTGANG` | `1-0:2.29.0` |
| `STROM_REACTIVE_INDUCTIVE` | `STROM_BLINDARBEIT_POSITIV` | C = 3 is "Blindleistung positiv"; inductive/capacitive is a quadrant property |
| `STROM_REACTIVE_CAPACITIVE` | `STROM_BLINDARBEIT_NEGATIV` | C = 4 is "Blindleistung negativ" |
| `STROM_REACTIVE_INDUCTIVE_EXPORT` | `STROM_BLINDARBEIT_Q1`…`Q4` | the old pair were Vorschub registers mislabelled as export |
| `STROM_REACTIVE_CAPACITIVE_EXPORT` | (as above) | |

`ObisCode::TARIFF_FEHLERREGISTER` (63) was added alongside `STORAGE_UNUSED`.

### Fixed

- **`ObisCode` was not string-stable: `FromStr` → `Display` was not the
  identity.** Parsing defaulted the storage group F to 255 and `Display` always
  printed it, so `"1-0:1.8.0"` — the spelling MSCONS carries and people type —
  came back out as `"1-0:1.8.0*255"`. A reading written through one path and a
  correction written through another produced different keys for one channel,
  so the correction superseded nothing and both rows survived resolution.

  `Display` now writes the **reduced form**, omitting `*F` when F is 255 ("not
  applicable", IEC 62056-6-1 Annex A). A storage group that carries information
  is never elided: `1-0:1.8.0*1` keeps its suffix and stays a distinct code.

- **`ObisCode` could not be deserialised by any non-borrowing deserialiser.**
  `serde(try_from = "&str")` required the deserialiser to hand out a borrowed
  `&str`, so `serde_json::from_reader`, bincode, postcard and MessagePack all
  failed with `invalid type: string "1-0:1.8.0*255", expected a borrowed string`
  regardless of what the payload said — streaming a file of intervals was simply
  impossible. Replaced with a visitor-based `Deserialize`, and a `Serialize` that
  writes through `collect_str` without an intermediate `String`.

- **The OBIS parser accepted signed value groups.** `u8::from_str` accepts a
  leading `+`, so `+1-0:1.8.0*+255` parsed and then rendered under the canonical
  spelling — a second spelling entering through the front door. Value groups are
  now ASCII digits or nothing.

### Changed

- **BREAKING — `ObisCode`'s string and `serde` form is now `"1-0:1.8.0"`**, not
  `"1-0:1.8.0*255"`. Both spellings still *parse*, so archives written under
  either form still read; only what the crate *writes* changed. See "Migrating
  stored OBIS codes" below.

- **BREAKING — `IntervalResolution`'s `serde` form is now its ISO 8601 duration**
  — `"PT15M"`, `"P1D"`, `"PT300S"` — instead of the derived Rust variant names
  `"QuarterHour"`, `"Day"`, `{"Custom":300}`. This is the same defect one type
  over: the value had a canonical string *and* a parallel serde encoding, and
  the serde one was a variant rename away from silently invalidating stored
  data. ISO 8601 is an external standard, so the new form cannot be renamed by
  any refactor here. `Display` / `FromStr` are unchanged.

- `ObisCode` now derives `PartialOrd` + `Ord`, ordering lexicographically by
  value group (A, then B, … then F) — it is routinely used as a sort and merge
  key.

- The OBIS parser now tolerates surrounding whitespace and leading zeros, so
  `"  01-00:01.08.00 "` from an EDIFACT segment normalises onto `"1-0:1.8.0"`.

### Added

- **`ObisCode::normalize(&str) -> Result<String, ParseError>`** — canonicalise a
  raw string without building a value first, for consumers holding a database
  column, a CSV cell or an MSCONS segment. Idempotent.
- **`ObisCode::to_full_string()`** and the `{:#}` alternate format — the explicit
  six-group form `1-0:1.8.0*255`, for systems that demand it. Parses back to the
  same value.
- `ObisCode::STORAGE_UNUSED` (255) and `ObisCode::has_unused_storage()`.
- `ObisCode::MAX_LEN` (23) — the longest string a code can render as, for sizing
  a fixed-width column. The canonical form is at most 19.
- `ObisCode`'s `Display` now honours width, fill and alignment (`{:>13}`). It
  wrote straight to the formatter before, so padding was silently dropped and
  codes would not line up in a table. Rendering stays allocation-free.
- Gas constants `GAS_NORMVOLUMEN_UMGEWERTET` (`7-0:13.2.0`), `GAS_ZUSTANDSZAHL`
  (`7-0:52.0.22`) and `GAS_BRENNWERT_MONATSMITTEL` (`7-0:54.0.22`), plus a
  warning on `GAS_VOLUME_M3` and in `conversion`: `gas_m3_to_kwh_hs` expects a
  **Betriebsvolumen**. Passing an already-converted Normvolumen applies the
  Zustandszahl twice and overstates the energy by a few percent, silently.
- `tests/string_canonicalisation.rs` — stability, totality, idempotence and
  injectivity under `proptest`, for the string form and the `serde` form, over
  both `ObisCode` and `IntervalResolution`.
- An `obis::mako_semantics_tests` module pinning every value-group rule above
  against the EDI@Energy Codeliste, and README/rustdoc sections stating the
  three rules that are read wrong most often: direction is C alone, D is a
  Messart, E = 63 is the Fehlerregister.

### Migrating stored OBIS codes

Only rows written by this crate need rewriting, and only if you store the string
rather than the six groups. The change is a suffix strip:

```sql
UPDATE readings
   SET obis_code = left(obis_code, length(obis_code) - 4)
 WHERE obis_code LIKE '%*255';
```

Run it over every table that keys on the code, then rebuild any index or
materialised view derived from it. If you enforce the canonical shape with a
check constraint, update it to reject a `*255` suffix rather than require one.
Rows that already hold the short spelling are already canonical, and rows with a
real storage group (`*0`, `*1`, …) must be left alone — the `WHERE` clause above
does that.

For `IntervalResolution`, map the old tags with
`'QuarterHour' → 'PT15M'`, `'HalfHour' → 'PT30M'`, `'Hour' → 'PT1H'`,
`'Day' → 'P1D'`, `'Month' → 'P1M'`, `'Year' → 'P1Y'`, and `{"Custom":n}` →
`'PT{n}S'`.

## [0.15.0] — 2026-07-27

A deliberate hard cut. Several fixes could not be made compatibly without
leaving the wrong behaviour reachable, so they were made breaking instead.
Every change below is mechanical to apply; the table at the end maps each old
call to its replacement.

### Added

- **`calendar` module** — Europe/Berlin calendar arithmetic, the piece every
  consumer was otherwise reimplementing: `local_day`, `local_month`,
  `local_year`, `day_start_utc`, `day_end_utc`, `month_start_utc`,
  `month_end_utc`, `year_start_utc`, `year_end_utc`, `day_length`,
  `month_length`, `year_length`, `day_kind` (`DayKind::{Normal, ShortDay,
  LongDay}`), `intervals_in_day`, `intervals_in_month`, `intervals_in_year`,
  `intervals_between`, `days_between`, `shift_back_one_year`, `days_in_month`,
  `days_in_year`, `is_leap_year`, `to_berlin`, `berlin`. Rules come from the IANA
  tz database, so historical transitions are correct.
- **`error::ParseError`** — one error type for every `FromStr` in the crate,
  replacing `ObisParseError`, `IntervalResolutionParseError`, `ParseCodeError`
  and `LoadProfile`'s `type Err = String`. Opaque, with `type_name()`,
  `input()` and `expected_values()` accessors, so added context is never
  breaking. A decoder can now parse several types in one `?` chain.
- `LoadProfile::{ALL, CODES}`.
- `MeterInterval::berlin_day()` — the correct grouping key for daily aggregation.
- `Display` + `FromStr` for `Sparte`, `MeasurementUnit`, `QualityFlag` and
  `IntervalResolution`, plus `QualityFlag::as_str` (previously absent) and
  `ALL` / `CODES` constants. `IntervalResolution` uses ISO 8601 durations
  (`PT15M`, `P1D`, `PT300S`), so `Custom(n)` round-trips.
- `IntervalResolution::{fixed_seconds, nominal_seconds, is_fixed, is_calendar, to_iso8601}`.
- `AggregationConfig::{with_ht_window, default}`, `ResampleConfig::{new, default}`,
  `MeasurementSeries::{with_melo_id, with_resolution, worst_quality,
  has_unbillable_intervals, record_event_with_note}`.
- `ObisCode::{KAELTE_ENERGY, WASSER_KALT_VOLUME, WASSER_WARM_VOLUME}`.
- **V11 `ValidationRuleId::UnorderedSeries`** — `validate_intervals` is now
  order-independent (see Fixed) and reports an out-of-order input as its own
  warning-level finding.
- `tests/berlin_calendar.rs` and `tests/serde_representation.rs`; a `purity`
  check (justfile recipe + CI job) asserting no clock read, I/O or `unsafe`
  reaches non-comment source.

### Fixed

- **`resample()` bucketed `Day`/`Month`/`Year` on UTC boundaries.** A German
  settlement period starts at 00:00 Europe/Berlin — 23:00 UTC the previous day in
  winter — so the first hour of every day and every month was booked into the
  preceding period. This made every §13 StromNZV monthly Mehr-/Mindermengensaldo
  wrong, not only at the DST transitions. Buckets are now Berlin calendar periods.
- **`expected_count` assumed a fixed day length.** It now follows the bucket's
  real duration: 92 quarter-hours on the spring-forward day, 100 on the fall-back
  day. Previously a complete autumn day of 100 intervals was indistinguishable
  from 96, so a genuine four-interval gap was masked every October.
- **`IntervalResolution::as_seconds()` returned wrong values** for `Month`
  (30 days), `Year` (365 days) and `Day` (24 h, wrong twice a year). Replaced by
  `fixed_seconds() -> Option<u32>`, which returns `None` for all three rather
  than guessing, and `nominal_seconds()` for sizing only.
- **`HtHours::is_ht` mixed timezones** — the hour was read in Europe/Berlin but
  the weekday in UTC, so the hours either side of local midnight were classified
  against the wrong day at week boundaries. `AggregationConfig` now holds a
  `TariffWindow`, which reads both locally.
- **`ObisCode::WAERME_ENERGY` was `8-0:1.0.0` — medium 8 is cold water, not
  heat.** A heat register therefore reported `is_water() == true`,
  `is_heat() == false`, and inherited water's daily default resolution instead of
  hourly. It is now `6-0:1.0.0`, and a test asserts every named constant
  satisfies the predicate its name implies.
- **The annual projection always scaled to 365 days**, understating a leap-year
  Jahresprognose by one day (0.27 %). It now uses the target year's real length,
  and groups daily sums by Berlin calendar day.
- **`observed_days` was computed as `(last_to - first_from).whole_days()`**,
  which truncates: fourteen calendar days spanning the spring transition are 335
  hours, so integer division reported thirteen. The daily average — and the
  projection built on it — came out **7.7 % too high** for any window containing
  a spring transition. Now counted in Berlin calendar days via
  `calendar::days_between`. The same truncation affected the seasonal factor's
  `period_days`.
- **`compute_seasonal_factor` shifted the comparison window back by a fixed 365
  days**, which drifts by a day across a leap year and lands an hour off across a
  DST transition — so "the same two weeks of March" silently compared different
  windows. It now shifts by a calendar year (`calendar::shift_back_one_year`,
  29 February → 28 February), and divides the prior year's total by that year's
  real length rather than 365.
- **`IntervalLengthClass::seconds()` carried the same approximation bug as the
  old `as_seconds()`** — `Daily => 86_400`, `Monthly => 86_400 * 30`. The type
  was a near-duplicate of `IntervalResolution`, so it was removed rather than
  patched; the bug is gone by construction.
- **`validate_intervals` silently required pre-sorted input** — the one function
  most likely to be handed messy data. Unsorted input produced a cascade of
  spurious V01 gap and V02 overlap errors, which are billing-blocking, on data
  that was merely shuffled. The adjacency rules now evaluate in timestamp order
  regardless of input order, `interval_index` still refers to the caller's slice,
  and the disorder itself is reported once as V11.
- **`LoadProfile::Custom` did not round-trip**: `as_str()` emitted `"CUSTOM"` but
  `parse("CUSTOM")` returned `None`, so a stored `Custom` profile failed to read
  back. Caught by the new `ALL`/`CODES` consistency test.
- **`MeasurementSeries::new`, `record_event` and the SMGW fault checks read the
  system clock**, making construction non-deterministic — two series built from
  identical inputs never compared equal, so no storage layer could write a
  round-trip test. All timestamps are now parameters; the crate contains no clock
  read at all, enforced by CI.
- `MeasurementUnit`'s `serde` tag (`KILO_WATT_HOUR`) disagreed with its
  `as_str()` code (`KWH`). There is now exactly one string per value.

### Changed (breaking)

| 0.14 | 0.15 |
|---|---|
| `aggregate(&ivs, config)` | `aggregate(&ivs, &config)` |
| `AggregationConfig { ht_hours: HtHours { .. } }` | `AggregationConfig { ht_window: TariffWindow { .. } }` |
| `MeterInterval { obis_code: Some("1-0:1.8.0".into()) }` | `MeterInterval { obis_code: Some("1-0:1.8.0".parse()?) }` |
| `iv.parsed_obis_code()` | `iv.obis_code` |
| `res.as_seconds()` | `res.fixed_seconds()` (`Option`) / `calendar::day_length` |
| `IntervalResolution::from_seconds(86_400)` → `Day` | → `Custom(86_400)`; a calendar day is not 86 400 s |
| `ResampleConfig { source_interval_seconds: 900 }` | `ResampleConfig { source_resolution: IntervalResolution::QuarterHour }` |
| `MeasurementSeries::new(malo, obis, ivs, src)` | `MeasurementSeries::new(malo, obis, ivs, src, ingested_at)` |
| `series.worst_quality` (field) | `series.worst_quality()` (method) |
| `series.recompute_quality()` | removed — the value is always derived |
| `series.record_event(kind, actor)` | `series.record_event(kind, actor, occurred_at)` |
| `gw.hours_since_last_contact()` | `gw.hours_since_last_contact(now)` |
| `gw.is_communication_fault(hours)` | `gw.is_communication_fault(now, hours)` |
| `IntervalResolution`'s `Display` (`"900s (15-Minuten)"`) | ISO 8601 (`"PT15M"`); `label()` still gives the German name |
| `MeasurementUnit` serde tag `KILO_WATT_HOUR` / `CUBIC_METRE` | `KWH` / `M3` |
| `ObisCode::WAERME_ENERGY` = `8-0:1.0.0*255` | `6-0:1.0.0*255` |
| `IntervalLengthClass` | removed — use `IntervalResolution` |
| `detect_interval_length() -> Option<IntervalLengthClass>` | `-> Option<IntervalResolution>` |
| `DeliveryEvidenceInput::interval_class` | `::resolution`, typed `Option<IntervalResolution>` |
| `ObisParseError`, `IntervalResolutionParseError`, `ParseCodeError` | all replaced by `ParseError` |
| `ObisParseError(pub String)` field access | `err.input()` |
| `ParseCodeError { type_name, input, expected }` fields | `err.type_name()`, `err.input()`, `err.expected_values()` |
| `<LoadProfile as FromStr>::Err = String` | `= ParseError` |
| `ValidationRuleId` | gained `UnorderedSeries` (V11) — breaking for exhaustive matches |

Callers that previously grouped by `iv.from.date()` should move to
`iv.berlin_day()`; those that assumed 96 intervals per day should call
`calendar::intervals_in_day(day, IntervalResolution::QuarterHour)`.

`MeasurementSeries` gained `PartialEq`/`Eq` (as did `ProvenanceEntry`), so
storage round-trip tests can assert equality directly. No `Default` impl was
added: `MeasurementSource` has no meaningful default, and inventing one would put
a fabricated provenance record into every test fixture. Use `new(...)` — now
total and deterministic — with the `with_*` builders.

### Enum exhaustiveness policy

Domain enums stay **exhaustive**; only error enums are `#[non_exhaustive]`
(currently `VirtualMeterError`). The reasoning is in the crate docs and the
README: a consumer mapping this crate's vocabulary onto their own storage codes
should get a build break when a new `Messtyp` or `SubstituteMethod` appears, not
a silent wildcard that files the reading under the wrong one.

The trade-off, stated so it is not a surprise: **adding a variant to a domain
enum is a breaking change here** and will be released as one.

## [0.14.0]

Initial published version.
