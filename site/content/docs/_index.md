+++
title = "Documentation"
description = "Guides for the metering crate: the Europe/Berlin calendar, meter readings, validation, Ersatzwertbildung, tariff registers, gas conversion and the regulatory basis behind each."
sort_by = "weight"
template = "section.html"
page_template = "page.html"
+++

`metering` computes **quantities** — kWh, m³ and kW — for the German energy
market. It performs no I/O, spawns no runtime, never reads the system clock, and
never routes a metered quantity through a floating-point number.

These guides explain the parts that carry domain knowledge you cannot infer from
the type signatures. For the full API, see
[docs.rs/metering](https://docs.rs/metering).
