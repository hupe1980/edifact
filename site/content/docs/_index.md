+++
title = "Documentation"
description = "Guides for edifact: parsing, writing, typed derive macros, dates and times, streaming, validation, segment groups, profile packs, CONTRL acknowledgements, diagnostics, and performance tuning for EDIFACT in Rust."
sort_by = "weight"
template = "docs-section.html"
page_template = "docs-page.html"
+++

From a first parse to production tuning. Rust snippets are doctests: compiled
and run, except `no_run` blocks (file and network I/O), which are compiled
only, and one `ignore`d fragment.

New here? Read [Getting Started](@/docs/getting-started.md), then
[Core Concepts](@/docs/core-concepts.md): segments, data elements and the `UNA`
service string advice.

Reading untrusted input? The
[threat model](https://github.com/hupe1980/edifact/blob/main/crates/edifact/THREAT_MODEL.md)
states what the crate defends against, what it leaves to you (size the
`ReaderConfig` limits to your traffic; treat values as data), and the evidence
for each claim.
