# Changelog

All notable changes to `metering` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the crate follows
semver, with the `serde` representation explicitly in scope (see the crate docs).

## [0.21.0] — 2026-08-30

A round of consumer feedback from a workspace that allocates the
Netzgangzeitreihe of a charging Übergabestelle to sessions and fixed devices,
and then builds MaBiS BK-Summenzeitreihen from the result. Six requests; five
implemented, one already in the crate under another name.

The theme is **conservation**. Two operations sit either side of the settlement
grid — a total spread across slots, and a slot split across claims — and both
had to be arranged so that `Σ = the thing that was divided` is a theorem rather
than a check. Doing that properly meant giving the crate a single allocation
primitive and a single grid, instead of one per caller.

Every removal is a hard cut; no deprecated shims.

### Added

- **`session` — a charging session or device log placed on the metering grid.**
  `split_session(from, to, energy, &samples, &config)` returns one
  `MeterInterval` per grid slot, contiguous and ascending, summing to the
  session total **exactly**.

  Clock-aligned register readings (OCPP `MeterValues` on the
  `ClockAlignedDataInterval`) are used where they land on slot boundaries and
  the resulting slots come back `Measured`; spans that straddle a boundary are
  divided by wall-clock time and come back `Estimated`. That distinction is the
  point of the function: a supplier settling the energy needs to know which
  quarter-hours were measured and which were inferred from a constant-power
  assumption the session did not obey.

  The total cannot drift because the slots are **differences of a cumulative**,
  not a sum with a correction on the last one — so the series telescopes to
  `cum(end) − cum(start)`, whatever rounding the cumulative itself needed.
  Truncation toward zero preserves the order of a non-decreasing function, so no
  slot can come back negative. A register running backwards, a sample outside
  the span and a total the samples contradict are errors, not something absorbed
  into the nearest slot.

- **`allocation` — one pool across many claims, with the residual reported.**
  `allocate(total, parts, basis)` guarantees `Σ allocated + residual = total`,
  exactly, for every key. `AllocationBasis::Fraction` reads a weight as an
  absolute share (the § 42b constant key); `Proportional` normalises it (the
  § 42b proportional key, and anything else with a ratio). An
  `AllocationPart::capacity` is the `Pos()` ceiling of the BDEW Anwendungshilfe.

  Nothing redistributes the residual — no largest-remainder pass. Under § 42b it
  is the generation that fed the public grid, and turning it into a rounding
  correction on somebody's invoice would credit them energy they did not
  receive.

  `validate_key` runs the same checks without dividing anything, so a persisted
  allocation rule can be rejected when it is *stored* rather than a month later
  in a settlement run.

- **`ids::Eic` and `ids::EicType`** — the sixteen-character ENTSO-E Energy
  Identification Code, with its **check character enforced at the parse**.
  Unlike the BDEW-Codenummer, whose Bildungsvorschrift exempts GS1-issued GLNs,
  the EIC scheme has no carve-out, so a mistyped code is rejected where the
  message that carried it is still available to report.

  Verified against the two worked examples in the ENTSO-E *EIC Reference
  Manual* §5.1 (`10X168Y4E6H0041Z`, `10X---ENTSOE---L`), the four German
  Regelzonen and the German bidding zone. `issuing_office()` is a `&str` and
  `object_type()` an `Option<EicType>`: the LIO list and the type list are
  ENTSO-E's to extend, and a library that hard-fails on an entry added after its
  release rejects data the market has already issued.

- **`Direction`, `MeterInterval::direction()` and `ObisCode::direction()`** —
  the flow direction as one value instead of two booleans that can both be
  `false`. `None` covers both "no OBIS code" and "this register has no
  direction" (Blindarbeit, a gas volume, a Zustandszahl).

- **`aggregation::sum_by_direction` → `DirectionalEnergy`** — the conservation
  check for a bidirectional Zählpunkt: `NGZ_import − Σ alloc_import` is a
  subtraction. Three buckets, so an undirected interval is counted rather than
  dropped, and `net()` is `import − export`. It counts **every** interval,
  billable or not, because it answers a physical question where `aggregate`
  answers *"can this period be invoiced"*.

- **`MeasurementSource::ChargeDetailRecord` / `ClockAlignedMeterValue` /
  `DeviceLog`** — provenance for session-derived energy, so a series built from
  a CDR is never mistaken for one that came from the MSB.

- **`DayBoundary::bilanzierungsmonat(year, month)`** — the settlement month
  addressed the way the market addresses it (*"Juni 2021"*), as a half-open UTC
  range, on either boundary. EDI@Energy *Allgemeine Festlegungen* v6.1c Kap. 3.1
  is quoted on the type and defines both. With it, `day_range_utc`,
  `month_range_utc` and `year_range_utc` for a period identified by a date.

- **`DayBoundary::bucket_bounds(instant, resolution)`** — *"which slot of this
  resolution contains this instant"*, now public and now the **only**
  implementation of that question. `resample` and `split_session` share it; a
  second copy would drift and the two would then disagree about which slot a kWh
  belongs to.

- **`session::merge_sessions`** — add several series that share a grid, slot by
  slot, with **union** semantics. `compute_virtual_meter`'s `Sum` intersects,
  because a missing source there means the total would be wrong; a charge point
  that was idle contributes no intervals and that absence *is* zero. It is a
  separate function rather than a flag because choosing the wrong one silently
  produces a plausible number. Grouped by `(from, to, obis_code)`, so a
  bidirectional point's two registers do not collapse into one total; only
  touched slots appear, because inventing zeros is `fill_gaps`, which records
  what it invented.

- **`ids::Regelzone`** — the four German Regelzonen, with the letter each
  occupies position 4 of a Bilanzierungsgebiet EIC with, and the ENTSO-E
  control-area code for each. **`Eic::regelzone()`** reads it back off a code.

  BDEW *Anwendungshilfe Energy Identification Codes* v1.0 (18.12.2017) §2.2.2
  gives Bilanzierungsgebiete their own Bildungsvorschrift — a `Y` code in the
  EIC function *Metering Grid Area* whose position 4 identifies the Regelzone
  (`N` TenneT, `R` Amprion, `V` 50Hertz, `W` TransnetBW). An EIC function is
  registry metadata and is not encoded in the code, so a `Y` code cannot in
  general be told apart from a Bilanzkreis's — but this one can, because the
  same section's Praxishinweis says the Energie Codes und Services GmbH
  *excludes* those four letters at position 4 for every other `Y` function.
  §2.2.1 also pins `11` as the German LIO, which is now `Eic::GERMAN_LIO` and
  `Eic::is_german()`.

- **`MeasurementPoint::bilanzkreis` and `::bilanzierungsgebiet`**, both
  `Option<Eic>`, so a MaBiS Summenzeitreihe groups on a check-character-
  validated value instead of free text. `EicType` is deliberately not
  constrained on either: a Bilanzkreisverantwortlicher gets a `Y` code in the
  EIC function *Balance Group*, older Bilanzkreise carry an `X` code, and the
  ENTSO-E manual explicitly keeps that national usage valid for Germany.

- **`EnergyFlow::direction()`**, **`MeasurementPoint::direction()`** and
  **`MeasurementPoint::direction_conflict()`**.

### Changed — breaking

- **`ALLOCATION_DP` and `allocation_share` moved to the new `allocation`
  module** and are re-exported at the crate root as before.
  `compute_community_allocation` is now `allocate` applied once per
  quarter-hour, and `compute_ggv_allocation` shares the same cut and cap, so the
  `Pos()` operator and the residual are defined in exactly one place.
- **`VirtualMeterError::InvalidFractions` → `VirtualMeterError::Allocation`**,
  wrapping `AllocationError`. What counts as a valid key is now defined once, by
  the module that applies it.
- **`CommunityInterval::surplus_to_grid` is `total − Σ allocated`, unclamped.**
  It was clamped at zero, which could only ever hide a contradiction: with a
  non-negative generation the clamp never fired, and with a negative one it
  broke the identity the field exists to satisfy.
- **A negative `Proportional` weight is refused.** It does not merely take
  nothing — it shrinks the denominator and so **inflates** every other part's
  share. A silent over-allocation is precisely what this arithmetic exists to
  make impossible.
- **`MeterInterval::is_import_energy` / `is_export_energy` removed** in favour
  of `direction()`. Two booleans that can both be `false` cannot distinguish
  "this register has no direction" from "it counts the other way", and callers
  read the first as the second.
- **`MeasurementPoint::is_bezug` / `is_einspeisung` removed**, likewise, in
  favour of `direction()`. A gas volume at a `Bidirectional` point made both
  `false`, which said nothing.
- **`ObisCode::is_einspeisung` removed** — a second name for `is_export`, which
  is the duplication this crate deleted `SharingReadiness::label()` for.
  `ObisCode::direction()` is now the primitive and `is_import` / `is_export`
  are derived from it, so there is one implementation of value group C rather
  than three.

### Fixed

- **`split_session` scanned the whole segment list for every slot.** The
  cumulative lookup and the per-slot quality check were both `O(slots ×
  segments)`, so a day-long device log sampled every minute did ~138 000
  comparisons for an answer that needs ~1 500. Segments and slots both run
  forward, so one cursor now walks the segment list once across the whole
  session. Found by writing the case down, not by a benchmark; the dense-log
  test pins the result.
- **`MeasurementPoint` resolved a direction contradiction in silence.** The
  point states direction twice — as a directed OBIS code and as declared
  `EnergyFlow` — and the old predicates simply preferred the code. That
  redundancy cannot be removed, because `EnergyFlow` also distinguishes storage
  from load and marks a four-quadrant meter as `Bidirectional`, neither of which
  any OBIS code says. So the disagreement is now **reportable**:
  `direction_conflict()` returns `Some((measured, declared))`.
- **`SessionError::SampleTotalMismatch`'s message** described only one of the
  two situations it covers. It now states both numbers and says the two
  disagree, which is true whichever way round they are.

### Not added, and why

- **A `Zaehlpunktbezeichnung` type.** It exists: `MeloId` **is** the
  33-character Zählpunktbezeichnung of VDE-AR-N 4400 / DVGW G 2000. A
  Messlokation is identified by its Zählpunkt; the two names describe the same
  characters, and giving them two types would produce two columns holding one
  identifier. The module docs now say so in as many words.
- **A `direction` field on `MeterInterval`, or a `BidirectionalInterval`.** The
  direction is already in value group C of the OBIS code, and MSCONS carries one
  time series per register. A field would be a second, separately-mutable copy
  of a fact the type already holds — the same objection that keeps the *unit*
  off it — and it would grow the hottest type in the crate. `direction()` reads
  it; `sum_by_direction` balances it.
- **Signed intervals.** A negative kWh here means a Korrekturenergiemenge
  (EDI@Energy *Codeliste* v2.5c §2.1). Overloading the sign to mean "reversed
  flow" would make those two indistinguishable, and no amount of documentation
  fixes an ambiguity in the data itself.

### Tests

- `tests/quantity_invariants.rs` gains the two conservation laws (a session
  split conserves its total and tiles its span; an allocation conserves its
  pool), the directional partition, and the **exactness claim tested directly**:
  a slot that comes back `Measured` carries exactly the difference of the
  register readings on its own boundaries.
- `tests/order_independence.rs` gains `allocate`, `split_session`,
  `merge_sessions` and `sum_by_direction` — the four new entry points that
  promise it.
- The merge proptest found a documentation gap rather than a defect: two
  sessions with an idle hour between them leave that hour out, and the docs now
  say so and point at `fill_gaps`.
- `tests/string_canonicalisation.rs` gains the EIC: round-trip, injectivity,
  case normalisation, and that **every** single-character corruption of the body
  is caught (the weight is in 2..=16 and 37 is prime, so the shift is never a
  multiple of the modulus).
- `tests/code_contract.rs` gains `Direction`, `AllocationBasis` and `EicType`;
  `tests/serde_representation.rs` pins their tags and the new
  `MeasurementSource` shapes.

## [0.20.0] — 2026-08-30

Order independence, which the crate documents in five places, turned out to be
false in two of them — and both failures needed a **tie** to show, which is why
no example-based test had ever caught them. A `Vec<MeterInterval>` arrives
shuffled every time it is merged from two MSCONS files or read back without an
`ORDER BY`, so the promise is one consumers rely on without noticing. It is now
a proptest suite rather than a sentence.

Alongside that: a channel classifier that contradicted its own file's
documentation, a daily length check that had never learned about the Gastag the
crate added in 0.19.0, a stuck-meter finding that reported the threshold instead
of the run, and a mandatory dependency on a system entropy source in a library
whose first line is that it reads no ambient state.

Then a round of consumer feedback from a home-energy-management workspace, which
brought § 14a EnWG's two control quantities, a conformance check for the
hundreds of DSO tariff calendars such a system curates, and a community-level
view of the § 42b allocation. Writing the property test for that last one
uncovered a **false exactness claim** that had been in the crate since 0.19.0.
Three of the seven requests were declined, with reasons, in the notes below.
Every removal is a hard cut; no deprecated shims.

### Fixed

- **Instants and dates were unreadable and unqueryable on the wire.** Every
  `OffsetDateTime` in the crate travelled as `time`'s own nine-element tuple —
  `[2026, 152, 12, 0, 0, 0, 0, 0, 0]`, the year, the **ordinal day**, the clock
  and the offset. That is a stable and deliberately compact choice on `time`'s
  part, and it is unusable as a *stored* format: `WHERE from > '2026-06-01'` has
  no meaning against an ordinal tuple, no JSON Schema, OpenAPI document or
  Parquet column type recognises it, and nobody can read it in a log. A crate
  that documents its timestamps as always-UTC and expects them in databases and
  topics owes them the spelling those systems speak.

  An instant is now `"2026-06-01T12:00:00Z"` (RFC 3339) and a calendar date
  `"2026-06-01"` (ISO 8601) — **in a human-readable format**. In a binary one
  they keep the compact tuple, because `MeterInterval` carries two instants and
  is the hottest type here. `time`'s own `serde-human-readable` feature splits
  the same way and is not used: its string is `2026-06-01 12:00:00.0
  +00:00:00`, which is readable but is not RFC 3339 and so is not what a
  `TIMESTAMPTZ` cast or a `format: date-time` validator expects. The split lives
  in a private `wire` module. **This is a wire-format break for anyone storing
  timestamps.**

  The crate had been paying for `time/serde-well-known` — whose only
  contribution is the `time::serde::rfc3339` module — and using nothing from it,
  while that feature also pulled `time`'s serde into builds that had not asked
  for serde at all. `time/serde` now sits behind this crate's own `serde`
  feature.
- **The hot types could not survive a binary format, which the crate claimed
  they could.** `AggregationRule`'s docs said internal tagging costs
  bincode/postcard support and *"the hot types a binary format is chosen for
  (`MeterInterval`, `ObisCode`) are unaffected"*. `MeterInterval` carries a
  `Decimal`, whose default `Deserialize` calls `deserialize_any` — the one
  question a format without a self-describing wire cannot answer — so it did not
  round-trip through postcard at all. `rust_decimal/serde-str` moves it to
  `deserialize_str`. The JSON is byte-identical, because a `Decimal` already
  travelled as its exact string. `postcard` is now a dev-dependency and a test
  holds **both** halves of the trade-off, including that the internally tagged
  types still deliberately fail.
- **`assess_modul_3` passed a calendar whose Niedertarif was unreachable.** A
  wrapping band written as a single `22:00–06:00` window — the mistake
  `ZaehlzeitFenster::spanning` exists to prevent — matches nothing, so NT was
  declared, never applied, and every other rule passed: three registers listed,
  the day fully covered by the ST fallback, HT untouched. The verdict was
  `Conforms` on a broken DSO calendar. New finding
  `Modul3Finding::RegisterNeverReached`.
- **…and the *ganzjährig identisch* check missed a seasonal NT/ST swap.** It
  compared only the Hochtarif and the uncovered minutes across the twelve
  months, so restricting the Niedertarif to the winter half — leaving the
  fallback to cover the summer — moved neither number and went unreported. The
  comparison is now over the whole per-register profile.

- **Three doc comments promised an exactness the arithmetic does not deliver.**
  Found by stating the identity and generating the numbers, which is the only
  way any of them could have been found — each needs a quotient that does not
  terminate, and a human writing an example picks numbers that divide cleanly.
  - `allocation_temperature` said *"the division is exact"*. The **weights**
    are exact — eighths, so the formula is `(8ϑ + 4ϑ + 2ϑ + ϑ) / 15` in
    integers and no float is involved — but `8/15` does not terminate, so
    `ϑ_allok × 15` does not recover the numerator. Left at full width, because
    its only consumer is `h_value`, which crosses into `f64` at once; the doc
    now says so and says to round before printing.
  - `UnitScale` said multiplying before dividing *"keeps the result exact"*. It
    keeps the **defining identities** exact — 3.6 GJ is 1 000 kWh and 3.6 × 10⁶ J
    is 1 kWh, to the digit, where a stored `277.777…8` factor would round twice
    and systematically — and rounds everything else once, at the end.
  - `AnnualForecast` rounded `projected_annual` and both interval bounds to
    three places and **said so nowhere**. Now [`FORECAST_DP`], documented, with
    the consequence named: a cut quantity is homogeneous only to its last
    reported place, so doubling every reading doubles the projection to within
    `2 × 10⁻³` kWh rather than exactly.

  The crate now states the contract once, at the top: **"exact" means no float,
  and one rounding at most.** Sums, differences and products of quantities do
  not round, which is why the conservation laws hold to the digit; division is
  where a choice has to be made, and the rule for making it is written down.
- **`QualityFlag::severity_rank` is a strict total order.** `Corrected` and
  `Substituted` shared rank 2, and `Faulty` and `Unknown` rank 5. `worse_of`
  keeps `self` on a tie, so the **worst quality of a set depended on the order
  the set arrived in** — a resampled bucket, a virtual meter, a differenced
  Lastgang and `MeasurementSeries::worst_quality` all took whichever of two
  equally-ranked flags the caller happened to list first. The eight flags now
  have eight distinct ranks, ordered by distance from a measurement:
  `Corrected` (2) has a measurement behind it and `Substituted` (3) does not;
  `Faulty` (6) is known bad and `Unknown` (7) is not even that.
- **`aggregate` breaks a tied peak by the earliest interval.** A flat load
  reaches its maximum in many quarter-hours, and `spitzenleistung_at` reported
  whichever of them came first *in the slice*. The Leistungspreis is the most
  disputed line on an RLM invoice, and "when?" is not allowed to depend on how
  the series was sorted on the way in.
- **`ObisCode::default_resolution` no longer contradicts the file it lives
  in.** `7-0:54.0.22` is documented three paragraphs above it as a **monthly**
  Brennwert mean — value group E selects the averaging period, 16 hourly,
  20 daily, 22 monthly — and was reported as an hourly channel. Separately, the
  reactive branch ignored value group D altogether, so `1-0:5.6.0` claimed a
  quarter-hour grid while `1-0:1.6.0`, the same Maximum register one Messgröße
  over, correctly claimed none. The function is now structured by medium and
  Messart: a Maximum (D = 6), a Vorschub (D = 9), a Momentanwert (D = 7) and
  the Fehlerregister are not equidistant series and answer `None`, active and
  reactive alike.
- **V06 knows a daily gas series is cut at 06:00.** The DST allowance that
  keeps a 23- or 25-hour daily interval from drawing a length warning tested for
  a Berlin *midnight*, so a daily series on the Gastag — the whole point of the
  `DayBoundary` added in 0.19.0 — drew a V06 warning on both transition days
  every year, for being exactly right. `ValidationConfig::day_boundary` and
  `ValidationConfig::on` carry the choice, and `QualityConfig::for_sparte(Gas)`
  sets it.
- **V05 reports the run, not the threshold.** The finding fired the moment the
  configured threshold was crossed and put *that* number in the message, so a
  meter frozen for three weeks was reported as "4 consecutive zero intervals" —
  the figure a reader acts on, wrong by two orders of magnitude. Each run is now
  reported once, when it closes, carrying the length it actually reached.
- **`ValidationConfig::enabled_rules` admits that `zero_run_threshold: 0`
  switches V05 off.** It reported the rule as armed, so a clean report claimed a
  stuck meter had been looked for when nothing had looked.
  `ValidationRuleId::enabling_field` now names `zero_run_threshold` for V05 and
  `negative_energy_is_error` for V03, the crate's two opt-*out* rules.
- **The GGV allocation identity was false, and is now true.** `GgvInterval`
  documented `consumption == allocated + net_grid_draw` as holding *"exactly:
  all three are `Decimal`"*. For the proportional key it did not. The share is
  a quotient — `consumption ÷ Σ consumption × generation` — carrying up to 28
  significant digits, so the `consumption − allocated` that follows needed more
  than a `Decimal` has and rounded: the identity came back a few 1e-27 kWh
  short. `ALLOCATION_DP` now cuts the derived share to six decimal places
  **toward zero**, which makes the identity exact, makes every share a number
  that fits in an invoice or an MSCONS field, and — because truncation only
  lowers a share — keeps `Σ allocated ≤ generation` so the § 42b Abs. 5 ceiling
  stays a theorem rather than becoming a clamp. A proptest over generated
  communities is what caught it; a unit test with round numbers never would.
- **§ 42c EnWG's dates.** The `sharing` module said Energy Sharing had been
  *"in force since 1 June 2026"*. The section itself entered into force on
  **22 December 2025** (BGBl. 2025 I Nr. 347); 1 June 2026 is when Abs. 4's duty
  on the Netzbetreiber begins. The distinction decides whether a delivery point
  that is `NotCapable` today is in breach of anything.
- **`AnnualForecast::seasonal_correction_applied` records whether the
  correction ran**, not whether the factor differs from 1. A prior year whose
  matching window happens to sit at the overall daily rate yields a factor of
  exactly `1` — a correct, fully corrected projection that reported itself as
  uncorrected, which is precisely the flag a caller uses to refuse to bill.

### Changed

- **`uuid` is no longer a dependency.** It existed for a single field —
  `MeasurementSource::RetroactiveCorrection { correction_id }` — that the crate
  never constructs, parses or validates, while every other external reference
  in the same enum is a `String`. It also pulled `getrandom` and its system
  entropy source into a library whose headline guarantee is that it reads no
  ambient state, and broke `wasm32` consumers for nothing. The field is now
  `correction_ref: String`. Four runtime dependencies remain.
- **`Zaehlzeitdefinition::split_energy` returns `BTreeMap<Option<&str>, _>`.**
  The keys borrow from the definition, so they are exactly the strings
  `registers()` lists and a lookup reads `split[&Some(HT)]` instead of
  `split[&Some(HT.to_owned())]`. A year of quarter-hours used to allocate a
  `String` per interval to produce three distinct keys.
- **`GasConversionParams::default_erdgas_h()` is gone**, replaced by
  `new(hs, z)` and `already_converted(hs)`. A Brennwert is operator data
  published per supply area and billing period, and it is a direct multiplier on
  a billed quantity: a 10.55 stand-in against a real 11.20 understates every gas
  invoice in the portfolio by 6 %, silently. It was the one place the crate
  invented a device property, against its own stated rule.
  `already_converted` is the useful half — it pins the Zustandszahl to 1 for a
  Normvolumen, where applying the real one a second time overstates the energy.
  The type also gained `Copy`, `PartialEq`, `Eq`, `Hash` and `serde`.
- **`sharing::combine` is `combine_readiness`** and is re-exported at the crate
  root, where the two functions producing its arguments already were.
- **`sharing::Finding` is exhaustive.** It was `#[non_exhaustive]`, the
  treatment this crate reserves for *errors*; a finding is routed, stored and
  displayed, and marking it also contradicted the `ALL` / `CODES` contract that
  promises exhaustive iteration. Exactly three types now carry the attribute,
  and all three are failure vocabularies.
- **`SharingReadiness::label()` is gone.** It returned the same string as
  `as_str()`, variant for variant — a second copy of a fact, in a crate whose
  design rules forbid one.
- **`mindestleistung_direktansteuerung` returns `Option<Decimal>`**, and
  `mindestleistung_ems` returns `None` for a set containing something that is
  not a steuVE. Ziff. 2.4.1 admits the four Fallgruppen only *"mit einer
  Netzanschlussleistung von mehr als 4,2 Kilowatt"*, so a smaller device is not
  counted by `n_steuVE` — and including one raises the floor for every other
  device in the set, quietly costing the Netzbetreiber reduction headroom it is
  entitled to. `SteuVe::is_steuerbar()` and `STEUVE_SCHWELLE_KW` say why; the
  latter is the *other* 4,2 kW, a different provision from
  `MINDESTLEISTUNG_KW` that happens to carry the same number.
- **`Modul3Context::at_delivery_point(bool, bool, bool)` is gone**, replaced by
  `with_modul_1`, `with_registrierende_leistungsmessung`,
  `with_intelligentes_messsystem` and the shorthand
  `at_a_conforming_delivery_point()`. Three positional booleans of which exactly
  one is an inverted condition is a call site nobody can read.
- **`sharing::Capability` is `Copy`** (and `Hash`), like every other verdict in
  that module. It is a discriminant plus an `EligibilityBasis`, which is itself
  `Copy`, and `basis()` already took `self` by value — so a caller who wanted
  both the basis and the verdict had to clone a two-word enum.
- **`ConversionError` is re-exported at the crate root**, like every other error
  type here. `normalize_to_kwh` was reachable as `metering::normalize_to_kwh`
  while the only error it returns needed `metering::conversion::ConversionError`.

### Added

- **`tests/order_independence.rs`** — proptest over `aggregate`, `resample`,
  `validate_intervals`, `score_intervals`, `fill_gaps`, `split_energy`,
  `to_lastgang` and `QualityFlag::worst_of`: a shuffled series gives an
  identical result. Half the generated series are drawn from a coarse half-kWh
  value grid, because a tie is what makes a maximum order-dependent and ties do
  not happen by accident in a fine-grained one — both defects above survive a
  generator that does not force them.
- **`code_contract::no_coded_enum_escapes_this_file`** — the contract file's own
  list said "adding an enum without adding it here is the only way to escape",
  which was a comment rather than a mechanism. The test now reads the crate
  source, collects every type `string_codes!` is applied to, and fails if one is
  missing from the list.
- **`IntervalResolution::from_observed_seconds`** — the tolerance table that
  maps a *measured* spacing onto a resolution, including the 23–25 hour band
  that makes a daily series a calendar `Day` rather than a fixed 86 400 s
  window. `classification::detect_interval_length` and
  `reading::detect_reading_cadence` held byte-identical copies of it, in a crate
  whose design rules forbid a second copy of a fact; both now call it.
- **`ValidationConfig::on(DayBoundary)`** and `ValidationConfig::day_boundary`.
- **A `para14a` module** — the two powers a § 14a netzorientierte Steuerung
  turns on, both in kW and both derived from nameplate figures, so quantities
  rather than money:
  - `mindestleistung_direktansteuerung` and `mindestleistung_ems` reproduce
    BK6-22-300 Anlage 1 Ziff. 4.5.1 / 4.5.2 verbatim, with the published
    Gleichzeitigkeitsfaktor table. Two traps in that formula are worth naming:
    its first term is `Max(0,4 × ΣP_WP; 0,4 × ΣP_Klima)`, a **maximum of two
    group sums** rather than `0,4 ×` everything — adding them overstates the
    floor on any installation carrying both a heat pump and room cooling, and a
    floor that is too high denies the Netzbetreiber headroom it is entitled to —
    and `n_steuVE` counts *all* controlled devices, Ladepunkte and Speicher
    included, not only the scaled ones.
  - `netzwirksamer_leistungsbezug` computes the share of Ziff. 2.3. That Ziffer
    *defines* the quantity and does not say how to split a grid draw local
    generation partly covered; VDE FNN's *Bewertung der Mindestleistung* (V1.0,
    April 2025) says the calculation is not its subject and points on to a text
    that is not freely citable. So the apportionment is a `Verursachungsregel`
    the caller picks, defaulting to the conservative one, with each convention's
    assumption written out — the treatment G 685's final rounding already gets.
- **`zaehlzeit::assess_modul_3`** — a conformance check against the BDEW
  *Anwendungshilfe für die Umsetzung von Modul 3* v1.1 (07.02.2025) §2, for
  refusing a bad DSO calendar before it reaches an optimiser. It checks the
  three tariffs, full coverage of the day, HT ≥ 2 h **per day class**, windows
  *ganzjährig identisch*, ≥ 2 billed quarters (not necessarily adjacent), a
  calendar-year validity, and the Modul 1 / iMSys / no-RLM preconditions.
  It deliberately does **not** check the HT and NT price corridors — this crate
  computes quantities, and it has no Arbeitspreis to compare against — nor the
  15.10. publication date, which is a Fristen question and which the AWH states
  for the first year rather than as a standing rule.
- **`Zaehlzeitdefinition::netzbetreiber`** and `published_by`. `id` is
  NB-assigned, so `HT/NT-1` from two operators are two calendars under one name.
  Deliberately **no** `year` field — that is `valid_from`/`valid_to` — and no
  `source` URL or hash, which is a property of the fetch, not of the calendar.
- **`ids::BdewCode`** and `CodeVergabestelle` — the 13-digit Marktpartner-ID,
  with the Bildungsvorschrift of BDEW *Identifikatoren in der
  Marktkommunikation* v1.2 §2.2 enforced. `MeasurementPoint::accountable_mp_id`
  and `MeasurementSource::Mscons`'s `sender_mp_id` are now typed rather than
  free text.

  The check digit is **reported, not enforced**: §2.3 names the same Lok- und
  Waggon-Verfahren as the MaLo-ID and then exempts GS1-issued GLNs, so a valid
  Marktpartner-ID may legitimately fail it. `has_bdew_check_digit()` says so
  without a library refusing data the market issued. The `MaloId` procedure,
  which has no such carve-out, stays enforced at the parse. That procedure now
  has one implementation serving both.
- **`compute_community_allocation`**, `AllocationKey`, `CommunityInterval` and
  `ParticipantAllocation` — the § 42b allocation for the whole community rather
  than one tenant at a time. The proportional denominator and the source index
  are formed once instead of once per tenant, so a year of quarter-hours across
  a twenty-flat building stops being quadratic in the tenant count; the surplus
  that fed the grid becomes a number rather than something to reconstruct; and
  the **§ 42b Abs. 5 pool ceiling becomes computable at all**, because it is
  defined over the whole participant set.

  It is reported (`CommunityInterval::pool_cap`) rather than applied: with
  fractions summing to at most 1 the per-participant `Pos()` cap already implies
  it, so clamping a second time would be a rule the statute does not contain.
  § 42c Energy Sharing uses the same arithmetic with a contractual
  Aufteilungsschlüssel — § 42c Abs. 3 Nr. 2 leaves the key to the contract and
  has no counterpart to Abs. 5, so `pool_cap` is an observation there rather
  than a ceiling.
- **`power_quality::PhaseApparentPower`**, `Phase` and `UNSYMMETRIE_LIMIT_KVA` —
  the VDE-AR-N 4100 Abschnitt 5.5.2 Unsymmetrieleistung, in **kVA**. The module
  refuses EN 50160's *voltage* unbalance for want of phase angles; load unbalance
  needs only three magnitudes and is a different question.
- **`tests/allocation_invariants.rs`** — proptest over generated communities:
  the three exact-arithmetic identities and the § 42b Abs. 5 ceiling.
- **Proptest coverage for the identifiers.** `BdewCode` joins the
  canonicalisation suite — round trip, injectivity, padding, the Vergabestelle
  read off the leading pair, and the advisory check digit — and `serde` is now
  asserted to write the same string as `Display` for `MaloId`, `MeloId` and
  `BdewCode`, which was pinned for `ObisCode` and `IntervalResolution` only.
- **`tags_added_in_0_20_are_pinned`** and four shape tests, so every type this
  release put on the wire is covered by the same semver commitment as the rest.
- **`tests/quantity_invariants.rs`** — the conservation laws and bounds of every
  arithmetic module, under generated input: differencing sums to the register
  difference, resampling preserves energy, a filled series is exactly its grid
  and every invented value is in the audit trail, the register split
  reconstructs the Arbeitsmenge, the Jahresprognose is its own formula, unit and
  gas conversion are exact where the quotient terminates and round once where it
  does not, the Allokationstemperatur is a bounded mean, Mehr and Minder are two
  halves of one signed delta, `P_min,14a` is monotone, the netzwirksamer
  Leistungsbezug is bounded by both sides, the Unsymmetrieleistung is a
  permutation-invariant spread, an EN 50160 outcome partitions its samples, and
  the § 42c decision table is total.

  This closes the last of the modules that were example-tested only. Every one
  of the three overstatements above, and the EN 50160 containment direction that
  turned out to be backwards in the *test*, came out of writing it.

### Build and CI

- **`postcard` is a dev-dependency with `default-features = false`.** Its default
  `heapless-cas` feature pulls `heapless` 0.7 and with it `atomic-polyfill`,
  which is unmaintained (RUSTSEC-2023-0089) and which the `cargo audit --deny
  warnings` lane rejects. Nothing here needs a `heapless::Vec`.
- **`.gitattributes` normalises every checkout to LF.** Two tests read the
  crate's own source — the coded-enum registry and the wire-format scan — and
  both now normalise line endings themselves as well: a `\r` in the middle of a
  line-shaped pattern makes such a scan report the wrong thing on one platform
  rather than fail loudly.

### Removed

- **The enum count in the README.** It said *"all thirty-four enums"*, which
  became forty-three this release. A hand-maintained tally of something the code
  already knows is exactly the second copy of a fact this crate's own design
  rules forbid; `code_contract::no_coded_enum_escapes_this_file` is the
  mechanism that keeps the list honest, and the prose now points at it.

### Not added, and why

Three requests were declined rather than half-built:

- **MiSpeL flow bookkeeping** (Abgrenzungs- and Pauschaloption). The arithmetic
  is quantity-shaped, but the quantities exist to size Umlageprivilegien (§ 21
  EnFG) and Marktprämien (§ 19 EEG) — the money boundary this crate draws — and
  the Arbeitsstand of 05.08.2026 still writes its own Bekanntgabe as
  `[01.10.2026]`, in square brackets, with part of it conditional on EU
  state-aid approval. A semver-stable library does not carry two regimes for a
  rule that is not yet a rule.
- **A `year` field on `Zaehlzeitdefinition`.** It is `valid_from` and
  `valid_to`. The crate's own design rule forbids a second copy of a fact.
- **Per-phase `MeterInterval`.** Asked for as L1/L2/L3 *active power* for the
  VDE-AR-N 4100 guard, and that guard is stated in **apparent** power: an
  inverter at cos φ < 1 — which VDE-AR-N 4105 requires it to be able to do —
  moves more kVA than kW, so a kW guard passes installations that breach the
  rule, and the error grows exactly when the grid asked for reactive support.
  The rule also covers only Erzeugungsanlagen, Speicher and Ladeeinrichtungen,
  so what the grid meter sees cannot answer it however many phases it carried.
  Tripling the crate's hottest type would not have delivered the check. The
  quantity lives in `power_quality` instead, where per-phase measurements
  already do.

## [0.19.0] — 2026-08-25

An audit of the invariants the crate states about itself, and of the two places
its DST correctness stopped short — followed by a round of consumer feedback
that found a rule which could not fire, a channel that could be mislabelled, and
twenty-eight enums whose wire form existed only through `serde`. Four defects were found where the code and
its own documentation disagreed, and one where a `serde` impl existed only at
compile time. The Gastag, added in 0.18.0 as a set of calendar functions,
becomes a boundary the daily calculations actually carry. Every removal is a
hard cut; no deprecated shims.

### Added

- **Every coded enum carries the whole contract**: `ALL`, `CODES`, `as_str`,
  `Display`, `FromStr`, and a `serde` tag that *is* the `as_str` code. The rule
  was already stated in the crate docs and held for six enums out of
  thirty-four; the other twenty-eight had a `serde` tag — so a wire form
  existed and consumers were storing it — with no way to reach that string
  without `serde`. `Debug` was the only handle left, and a rename upstream
  would have gone on writing rows, spelled differently, with nothing failing.
  `AnomalyKind` is the case that prompted it: it is the audit record for a span
  that could not be differenced, and a § 146 Abs. 4 AO trail built on `Debug`
  is a trail that can silently go missing.

  `CODES` is now *computed* from `ALL` in a `const` block, so the two cannot
  drift and a `CHECK` constraint generated from it cannot drift from what the
  crate writes. `tests/code_contract.rs` asserts all six properties for all
  thirty-four enums, and adding one without adding it there is the only escape.

  A **code** and a **description** stay separate where a type has both:
  `Holiday::as_str()` is `BUSS_UND_BETTAG` and `name()` is *"Buß- und Bettag"*;
  `RegisterUnit::as_str()` is `KILO_WATT_HOUR` and `symbol()` is `kWh`;
  `SubstituteMethod::as_str()` is `ZERO_FILL` and `description()` is the German
  prose an invoice annex prints.
- **`RuleSet`, `ValidationConfig::enabled_rules` / `disabled_rules`,
  `ValidationResult::evaluated` / `skipped`, `ValidationRuleId::enabling_field`,
  `QualityReport::evaluated` / `covers_every_rule` / `skipped_rules`.** Four of
  the eleven rules are opt-in — they need a grid spacing, an outlier threshold,
  a reference instant or a plant capacity, none of which this library will
  invent — and nothing said which were inert. A clean `ValidationResult` read
  the same whether a rule had run and found nothing or had never run at all.

  Concretely: `QualityConfig::for_sparte`, the documented way to configure per
  commodity, sets no `max_plant_power_kw`, so **V12 could not fire on that
  path** while a service built on it could describe V12 as an active
  Error-severity rule in its docs, its API responses and its operator guide.
  Declining to invent a nameplate capacity was right; leaving the inertness
  invisible was not. `disabled_rules()` answers before a run, `evaluated`
  after one, and the difference between the two is exactly "the data stopped
  it" rather than "the config did".
- **`compute_ggv_allocation` and `GgvInterval`.** `compute_virtual_meter`
  returns a GGV tenant's net grid draw; a § 42b or § 42c settlement is built on
  the **allocated** energy, and the only way to get it was
  `max(0, consumption − net)` — which meant re-reading and re-projecting the
  tenant's own consumption series purely to subtract it back out, and
  reproducing the `Pos()` cap in the caller where a change to it would never
  reach them. `GgvInterval` carries `consumption`, `generation`, `share`
  (before the cap), `allocated`, `net_grid_draw` and `quality`, with
  `capped()` and `surplus_to_grid()` derived from them. `consumption ==
  allocated + net_grid_draw` holds exactly in every interval, and
  `compute_virtual_meter` now *projects* from this result rather than
  recomputing, so the cap has one implementation.
- **`ObisCode::as_lastgang` / `as_zaehlerstand` / `as_vorschub`** — the Messart
  conversion a ZSG → Lastgang pipeline needs. `None` off the electricity axis
  (value group C is a Messgröße for gas, not a direction) and `None` for a
  tariff register, because the Codeliste v2.5c defines the Lastgang only as
  `1-b:1.29.0` and inventing `1-0:1.29.1` would silently collapse HT and NT
  onto one channel.
- **`reading::detect_reading_cadence`** — the median spacing of a
  Zählerstandsgang's timestamps. `detect_interval_length` medians each
  interval's *duration*, and a `MeterReading` is a point with no duration, so
  it could not answer "how often is this meter read?" — which is exactly the
  number `LastgangConfig::with_capacity_kw` needs.
- **`ResultChannel`** and `LastgangConfig::on_channel` — see the fix below.
- **`ValidationConfig::at_reference_instant`**, the builder for V08's `now`,
  matching `with_plant_capacity_kw` for V12.
- **`Rollover::duration`**, and re-exports for `consumption_between`,
  `detect_reading_cadence`, `ResultChannel` and `SlpValueTable`.
- **`calendar::DayBoundary`** — `Midnight` or `Gastag`, with the full period
  API on it (`day_start_utc`, `day_end_utc`, `local_day`, `day_length`,
  `intervals_in_day`, and the month and year equivalents). A Gastag is not a
  different *length* of day, it is the same day cut six hours later, so the
  choice is a boundary rather than a resolution — which is why it is not a new
  `IntervalResolution` variant, whose canonical string is an ISO 8601 duration
  and has nothing to say about phase.
- **`ResampleConfig::on` / `ResampleConfig::to_gas_daily`** and
  **`FillGapsConfig::on`** — the boundary carried into the two places a daily
  grid is actually built. It carries up to months and years too: a gas month
  runs 06:00 on the first to 06:00 on the first of the next, so a monthly gas
  total is a whole number of Gastage rather than a calendar month shifted.
- **`CustomSeconds`** — the opaque payload of `IntervalResolution::Custom`; see
  the breaking change below.
- **`QualityGrade: FromStr`**, `QualityGrade::CODES`, and the full
  `ALL` / `as_str` / `Display` / `FromStr` / `CODES` treatment for
  `MeterStatus` and `MeterLifecycleEventType`, which had none of it. A grade or
  a status written to a report column now reads back as itself, like every
  other code in the crate.
- **`MeterLifecycleEventType::breaks_register_continuity`** and
  **`MeterStatus::is_in_service`**.
- **`load_profile::SlpValueTable`** — a name for the 12 × 3 profile table.
- Re-exports at the crate root for the entry points that were reachable only
  through their modules: `assess_capability`, `assess_delivery`, `Finding`,
  `EligibilityBasis`, `MeteringCapabilityInput`, `DeliveryEvidenceInput`,
  `next_milestone`, `ROLLOUT_MILESTONES`, `RolloutMilestone`, `QuotaScope`,
  `voltage_percentile`, `exceedance_pct`, `gas_day_start_utc`, `local_gas_day`.
  `sharing` exported its enums and not the functions that produce them.

### Fixed

- **V07 now examines every fall-back day the series spans**, not only the one
  it starts on. The rule keyed off `local_day(first.from)`, so it fired for a
  single-day query and for nothing else: a month of MSCONS covering 25 October,
  an annual export, a MaBiS Summenzeitreihe — the deliveries where a collapsed
  repeated hour is least visible by eye — all passed it silently.
- **V01 reports any uncovered span, not only a whole missing interval.** A
  series of exactly-900-second intervals sitting off the grid
  (`00:00–00:15`, `00:20–00:35`, …) validated completely clean: V01 required a
  full `expected_interval_secs` before firing and V06 only measures each
  interval's own length, so five minutes of energy went unaccounted for in
  every slot with nothing to report it. A hole shorter than the grid says so in
  its message rather than claiming "0 intervals missing".
- **A skipped local time now resolves forward.** Turning a Berlin wall-clock
  time into an instant fell back to reinterpreting the naive time as UTC when
  the clocks had skipped it, which landed *before* the gap — `shift_back_days`
  asked for 02:30 on the spring Sunday and returned 01:30 local, while the doc
  claimed it returned "the instant the clock jumps". It is pushed forward by
  the length of the gap now (02:30 → 03:30), the convention `java.time`,
  `chrono` and Python's `zoneinfo` share, and the resolution happens in one
  place for `day_start_utc`, `gas_day_start_utc`, `shift_back_days` and
  `shift_back_one_year` alike.
- **`DynamicSlpProfile` can be serialised.** Its `values` field is a
  `BTreeMap` keyed by `(u8, SlpDayType)`, and a JSON object key has to be a
  string — so the derived impl compiled and then failed at run time with *"key
  must be a string"* on the format the licensed BDEW tables actually arrive in.
  The `serde` form is a list of `{ month, day_type, values }` records; the
  in-memory type is unchanged.
- **`LoadProfile::parse` trims.** Every other parser in the crate tolerates
  surrounding whitespace; this one rejected `" H0 "`, so a code arriving from a
  CSV cell or a fixed-width field was a parse failure rather than an H0.
- **`fill_gaps` labels substitutes with the earliest interval's OBIS code**,
  not `intervals[0]`'s. The input need not be sorted, and a label that depends
  on the caller's ordering is not a label.
- **`compute_virtual_meter`'s `Sum` takes the furthest contributor's end**,
  not the last one listed, so the result's interval length no longer depends on
  the order the ids appear in the rule.
- **A feed-in Zählerstandsgang is no longer relabelled as import.**
  `LastgangConfig::strom()` stamped a fixed `ObisCode::STROM_BEZUG_LASTGANG` on
  every interval it derived, so differencing a `1-0:2.8.0` series produced
  intervals labelled `1-0:1.29.0` — the values right, the channel a lie, and
  nothing downstream able to see it. `ResultChannel::Derived` relabels each
  interval from the register it actually came from, and is what `strom()` uses.

### Fixed (regulatory)

- **The `GHD` gas profile exists, and 0.18.0 was wrong to delete it.** That
  release removed `GasGHD` on the finding that *"no gas SLP is named GHD"* and
  made `parse("GHD")` a hard error — for a code the market uses. The EDI@Energy
  *Codeliste TUM- und BDEW-SLP Gas* v1.1 §6.3 lists
  *"Summenlastprofil Gewerbe, Handel, Dienstleistung"* under the TUM codes
  `HD3` / `HD4`, and the BDEW/VKU/GEODE Leitfaden publishes its SigLinDe row
  alongside the other fourteen: `GHD` is the *Stützpunkt* whose coefficients
  and weekday factors are a weighted mean over the eleven sector profiles, for
  a delivery point that fits none of them. `LoadProfile::GasGHD` is back, with
  `is_gas_aggregate()` to tell it from the sector types, and the gas profile
  count is **fifteen**.
- **`SigLinDe::DE_HEF34` no longer claims an EDI code.** Its doc read
  *"Ausprägung `+` (EDI code `1D4`)"*; `1D4` matches no code shape in the
  Codeliste, and the trailing `34` in the profile name is the SigLinDe
  **variant** (`33` and `34` differ in how much of the demand the linear part
  carries), not a Klasse/Ausprägung code. The coefficients themselves are
  unchanged and remain verified against the printed row by
  `h(8 °C) = 1.00000`.
- **Citations updated to the versions in force.** EDI@Energy *Codeliste der
  OBIS-Kennzahlen und Medien* **v2.5c** (was v2.4b) and *Allgemeine
  Festlegungen* **v6.1c** (was v6.1b), both binding since 01.04.2026; the
  BDEW/VKU/GEODE SLP-Gas Leitfaden **KoV XV, Stand 27.03.2026** (was
  28.10.2025). Every quoted passage was re-checked against the current text
  rather than carried forward.
- **`ValidationConfig::negative_energy_is_error` gains its primary source.**
  Codeliste v2.5c §2.1: *"Die Energieflussrichtung wird mittels der
  OBIS-Kennzahl definiert. Mit Ausnahme der Übermittlung von
  Korrekturenergiemengen (hier können die Werte auch negativ sein), sind die
  Mengenangaben nur mit positiven Werten oder 0 anzugeben."* The former
  `bidirectional()` rationale — "a bidirectional register" — was not the
  market's; direction lives in value group C, not in the sign.
- **`tests/regulatory_showcase.rs` pins the four day-start codings** the
  Allgemeine Festlegungen Kap. 3.1 prints — Strom `2300`/`2200`, Gas
  `0500`/`0400` — and the Bilanzierungsmonat worked example, which is the
  primary source for `DayBoundary`'s month behaviour.

### Changed (breaking)

- **`AggregationRule` is internally tagged** — `{"kind":
  "GGV_CONSTANT_ALLOCATION", "plant_melo_id": …}` — on
  `VirtualMeterKind`'s own spelling. The derived form was externally tagged
  with the Rust variant names, so one discriminator had two spellings
  (`GgvConstantAllocation` against `GGV_CONSTANT_ALLOCATION`) *and* two
  positions: a JSON key in one place, a JSON value in the other. Storing a rule
  as `jsonb` then meant a separate `rule_type` column, because a key cannot be
  indexed or queried as a value, and a recursive JSON path into the payload,
  because its depth depended on the variant.

  Stated plainly because it is a real trade: internal tagging needs a
  self-describing format, so `AggregationRule` will not round-trip through
  bincode or postcard. A rule is configuration, stored once per delivery point
  in a queryable document; the hot types a binary format is chosen for
  (`MeterInterval`, `ObisCode`) are untouched and stay format-agnostic.
- **`ValidationRuleId`'s `serde` tag is the `Vnn` code.** It serialised as
  `"GAP_DETECTED"` while `Display` wrote `"V01"` — one rule, two stored forms,
  neither able to read the other back. `code()` is renamed `as_str()` for
  consistency with every other coded enum; the string it returns is unchanged.
- **`Rollover` carries `from` and `to`**, not a single `at`. It and `Anomaly`
  describe the same shape — *what happened between two readings* — and are
  routinely logged into one audit table, where a row whose span was `from ==
  to` because the type only held one instant is not a span.
- **`LastgangConfig::result_obis: Option<ObisCode>` is now
  `result_channel: ResultChannel`**, with `Unchanged`, `Derived` and
  `Fixed(code)`. `labelled(code)` still sets `Fixed`.
- **`LastgangConfig::with_capacity_kw` takes an `IntervalResolution`**, not raw
  seconds — pair it with `detect_reading_cadence`. A **calendar** resolution
  derives the cap from the *longest* that period can be (25 hours for a day,
  366 days for a year): a ceiling must not reject a legitimate fall-back day,
  which a flat 24 would have done on every daily-read meter in the country.
- **`ProvenanceEventType` is `Copy` and `Hash`**, like every other coded enum.
- **`Sparte`, `Bundesland`, `MarktRolle` and `LoadProfile` accept input
  aliases** — `WÄRME`, `DE-BY`, `ÜNB`, and the pre-0.18 `EF`/`MF`. An alias is
  never written back and is deliberately absent from `CODES`.
  `MarktRolle::as_str()` is ASCII `UENB`; `abbreviation()` keeps the BDEW
  spelling `ÜNB`.
- **`IntervalResolution::Custom` carries an opaque `CustomSeconds`.**
  `Custom(900)` was constructible beside `QuarterHour`: two distinct values
  meaning one thing — two database keys for one 15-minute grid — and only one
  of them survived a round trip, since `Custom(900)` writes `"PT900S"` and
  `"PT900S"` parses back as `QuarterHour`. `CustomSeconds::new` refuses `0`,
  `900`, `1800` and `3600`, so `IntervalResolution::from_seconds` is the one
  way in and it normalises. The property tests could not see the hole because
  their strategy already went through `from_seconds`; they now assert it
  directly. `Custom(n)` in a pattern becomes `Custom(c) => c.get()`, and
  `Custom(7200)` in an expression becomes
  `IntervalResolution::from_seconds(7200).unwrap()`.
- **`MeterExchangeEvent::exchange_date` is a method, not a field.** It restated
  what `exchange_at` already determines and was free to contradict it —
  23:30 UTC on 14 June is already 15 June in Berlin. It is derived through
  `calendar::local_day` now.
- **`normalize_to_kwh` no longer accepts `"kvar"`.** Integrating a reactive
  power over an hour gives kvarh, not kWh; a function named `normalize_to_kwh`
  returning it put a kvarh figure in a kWh column with nothing to catch it.
  `"kW"` is unchanged.
- **`MeasurementPoint`, `MeterLifecycleEvent` and `MeterExchangeEvent` derive
  `PartialEq` / `Eq`.** Additive in practice, listed here because it adds
  bounds to the types' generic parameters — they have none, so nothing breaks.
- **`ResampleConfig` and `FillGapsConfig` gained a `day_boundary` field.**
  Struct-literal construction needs it; the constructors and builders default
  it to `Midnight`, which is the previous behaviour.

### Considered and declined

- **A `PlantCapacity` newtype shared by `LastgangConfig` and
  `ValidationConfig`.** The two *are* the same physical fact at two points in
  the pipeline — one prevents a bad value from being differenced into
  existence, the other flags one that arrived already formed — but a newtype
  over `Decimal` labels that relationship without enforcing anything: both
  already take kW, and there is nothing to mix them up with. What was missing
  was the second half of the ceiling, the interval it applies over, and that is
  now typed (`with_capacity_kw` takes an `IntervalResolution`). The two are
  cross-documented and a test asserts they agree on the same physical case.
- **A `capped: bool` field on `GgvInterval`.** It restates `share > allocated`,
  two numbers already in the struct, and a field that restates what is beside
  it is a field that can contradict it — the rule this release applied to
  `MeterExchangeEvent::exchange_date`. It is a method.

### Documentation

- `aggregate` no longer claims to sort its input. It never did, and does not
  need to: every quantity it computes is order-independent. A test now asserts
  that rather than the doc asserting it.
- `AggregationRule::PvSelfConsumption` documented three output series while the
  engine returned one signed net series. The doc states the one it returns and
  gives the three derivations that follow from it.
- The site's design page gains a **"No second copy of a fact"** section, which
  is the rule behind the derived register unit, the derived exchange date, the
  computed `worst_quality` and the audit trail that records what ran.
- The validation, calendar, gas, readings and Ersatzwerte pages are updated for
  the changes above, and every new snippet is mirrored in `tests/doc_samples.rs`.

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

- **The gas `LoadProfile` codes are now the real ones.** `GasEF`/`GasMF`
  (codes `EF`, `MF`) were not BDEW gas profile codes. The gas variants are now
  the published TUM/FfE types — `HEF`, `HMF`, `HKO` and the eleven Gewerbe
  profiles `GKO`, `GHA`, `GMK`, `GBD`, `GGA`, `GBH`, `GWA`, `GGB`, `GBA`,
  `GPD`, `GMF`. `parse` accepts `EF`/`MF` as lenient aliases.

  > ⚠️ **Corrected in 0.19.0.** This release also deleted `GasGHD` and made
  > `parse("GHD")` a hard error, on the finding that `GHD` "does not exist at
  > all". That was wrong — it is the Summenlastprofil Gewerbe, Handel,
  > Dienstleistung, TUM codes `HD3`/`HD4` — and 0.19.0 restores it. Anything
  > written against 0.18.0 that mapped `GHD` to an error should be revisited.
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
