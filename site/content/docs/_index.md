+++
title = "Documentation"
description = "Guides for the metering crate, from a first Series to the regulatory basis behind every quantity."
sort_by = "weight"
template = "section.html"
page_template = "page.html"
+++

`metering` computes the **quantities** of the German energy market — kWh, m³
and kW — with no I/O, no async and no clock. These guides explain the domain
knowledge the type signatures cannot carry, and cite the clause behind each
rule. The full API is on [docs.rs/metering](https://docs.rs/metering).

**Start** with a first series and the end-to-end example. **Concepts** are the
three things everything else rests on: the calendar, the series, the
identifiers. **Tasks** are the everyday pipeline; **Specialised** covers the
grid, allocation and EEG quantities. **Reference** holds the design
constraints and the regulatory basis.
