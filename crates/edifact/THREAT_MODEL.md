# Threat model

What `edifact` defends against, what it trusts and where its responsibility
ends, with every known threat class, its status and evidence, open ones
included. The same text is on docs.rs (`edifact::threat_model`). Report a
suspected vulnerability privately as described in
[SECURITY.md](https://github.com/hupe1980/edifact/blob/main/SECURITY.md).

## Release status

This describes `edifact` 0.20.0. Every status was checked against the code
and its tests. The per-feature models behind this summary were built with
[ThreatSpec](https://github.com/hupe1980/spec-kit-threatspec) (STRIDE).

`edifact` is a synchronous library for the ISO 9735 batch syntax. It
tokenizes, parses, writes, (de)serializes, validates and acknowledges
(`CONTRL`) interchanges. It contains no `unsafe` code
(`#![deny(unsafe_code)]`), performs no I/O of its own beyond the reader or
writer the caller passes in, and has three required runtime dependencies
(`memchr`, `smallvec`, `thiserror`).

It does **not** authenticate, sign, encrypt or replay-protect anything: the
ISO 9735 security segments (parts 5–9) are refused (`E056`), not
implemented.

## Adversary

- **A hostile sender**: whoever produced the bytes being parsed (a trading
  partner, a compromised partner system, anyone who can drop a file into an
  inbound channel). The adversary is adaptive and controls every byte,
  including the service string advice (`UNA`), the declared character set,
  counts, references and the stream's length and pacing.
- **Hostile values on the write path**: application data (names, free text,
  references) that reaches the writer and may contain EDIFACT service
  characters. This data comes from the caller's own users, not from the
  caller itself.

## Assets

- **Availability of the consuming process**: memory, CPU time, and the
  progress of a long-lived stream.
- **Integrity of what is accepted**: the segments, values, counts and
  references a caller acts on, such as booking a message or settling an
  invoice.
- **Integrity of what is written**: interchanges whose structure cannot be
  changed by the values inside them.
- **Integrity of acknowledgements**: a `CONTRL` must not accept what was
  rejected, and must not be addressed to the wrong party.
- **Integrity of diagnostics**: error text and reports that end up in logs,
  tickets and user interfaces.

## Trusted

- The calling program, the `ReaderConfig` / `RecoveryConfig` limits it sets,
  and the schemas, directories, guides and rule packs it passes in.
- The Rust standard library, the compiler, and the three runtime
  dependencies.
- The reader and writer the caller supplies, which behave as `std::io`
  documents.

## In scope

- Everything bytes from a hostile sender can do through every public entry
  point: tokenizer, parser (slice and reader), charset decoding, typed
  deserialization, envelope and layered validation, group resolution,
  recovering scan and `CONTRL` generation.
- Everything hostile values can do through the writer, the serializer and
  the `CONTRL` builder.
- What the crate reports: errors, spans, validation issues and diagnostics.

## Out of scope

- Transport security: authenticating the sender, confidentiality, integrity
  in transit and replay (AS2/AS4, SFTP, VAN). This is the transport layer's job.
- Business rules: whether a valid message is a legitimate one. This belongs
  to the caller, its implementation guides and its rule packs.
- Side channels, and bugs in the compiler or standard library.
- Denial of service within the configured limits, and the cost of collecting
  a whole interchange into memory, which is the caller's choice (E-01).
- What the caller does with the values it receives, for example in SQL,
  HTML or a shell.

## Strength

Every malformed, truncated or over-budget input becomes a typed
`EdifactError` with a stable code and, where one exists, a byte span. It is
never a panic, a silently skipped byte or a plausible partial result. This
is the crate's central security property. It is backed by the bolero
targets (each asserts an output contract, not just "no panic"), the
boundary and resource-limit suites, and the round-trip tests. Fuzzing in CI
is a smoke run (50 000 iterations per target, 1 000 000 for the
`fuzz_parity_*` targets) without a persistent corpus, so it is evidence, not
proof.

A parser that agrees with the standard is not a validator of intent. A
syntactically valid interchange can still be a malicious one, and only the
caller's guide and business checks can tell.

## Threat classes

Ids are stable and never reused. The fields are:

- **Sev**: impact if exploited. `high` means a hostile sender takes down or
  misleads the consumer; `medium` means the consumer is misled in limited
  conditions; `low` means hygiene.
- **Disposition**: `mitigate` means `edifact` is designed to prevent it.
  `harness` means the caller must handle it, with help from `edifact`.
- **Status**: `mitigated` is used only with test evidence and no known open
  variant. `partial` means some variants are open; they are listed under
  [Open variants](#open-variants). `open` means no control covers it yet.
- **Evidence**: tests in `crates/edifact/tests/` (`file::test`) that attempt
  the attack and assert the refusal. A leading `::` repeats the file named
  before it.

| Id | Class | Description | Sev | Disposition | Status | Evidence |
|---|---|---|---|---|---|---|
| E-01 | Resource exhaustion | Endless or huge input, giant segments, unterminated messages, endless whitespace or a flood of faults exhausts memory or never ends. Every entry point is bounded by default — `max_segment_bytes` 64 KiB, `max_message_bytes` 16 MiB (`UNH` through `UNT`), `max_input_bytes` 256 MiB, every consumed byte counted — and `ReaderConfig::unbounded()` is the one explicit opt-out. The recovering scan adds a fault budget. Heap grows per element: a held message window, or a collected interchange, takes at most 128 × its input bytes. | high | mitigate, harness | mitigated | resource_limits.rs::default_limits_are_the_documented_constants, ::every_default_entry_point_stops_endless_input, ::every_default_window_stops_unterminated_message, ::max_message_bytes_boundaries, ::unbounded_lifts_every_limit, ::max_input_bytes_exceeded_is_an_error_on_both_paths, ::max_input_bytes_never_yields_a_segment_past_the_cap, ::peak_heap_bounded_by_limits_unterminated, ::peak_heap_bounded_by_limits_long_lived, ::peak_heap_bounded_by_limits_decoding, ::decoding_paths_count_decoded_bytes, ::per_message_cap_on_long_lived_stream, ::endless_whitespace_reader_stops, ::recovering_scan_stops_at_fault_budget, ::recovering_scan_applies_max_message_bytes, ::limit_error_terminates_windows_and_typed_iterators, ::stray_terminators_count, ::unterminated_message_message_windows; allocation_budget.rs::worst_case_window_heap_within_factor, ::one_mb_peak_heap_reader_stream; boundary_conditions.rs::default_segment_limit_is_enforced_on_unterminated_input; bolero_harness.rs::fuzz_reader_and_slice_agree_on_limit_errors; docs_api_references.rs::read_entry_points_state_resource_bounds; docs_claims.rs::guides_quote_default_limits; interchange_header.rs::hostile_heads_fail_within_the_bound, ::read_from_consumes_little_of_a_large_interchange |
| E-02 | Crash on malformed input | A panic, overflow or out-of-bounds access on hostile bytes takes down the consumer. There is no `unsafe` code, and every entry point is fuzzed for an output contract. | high | mitigate | partial | bolero_harness.rs::fuzz_from_bytes_no_panic, ::fuzz_parser_output_is_well_formed, ::fuzz_parity_raw, ::fuzz_tokenizer_with_limit_no_panic, ::fuzz_validate_envelope_no_panic, ::fuzz_service_segments, ::fuzz_message_windows_no_panic, ::fuzz_recovering_scan_matches_strict, ::fuzz_directory_validator_no_panic, ::fuzz_profile_rule_pack_no_panic |
| E-03 | Parser differential | The slice path (`from_bytes`) and the reader path (`from_reader`) give different answers for the same bytes. One component accepts what another refuses, and an input can be shaped to pass a check and then be read differently. Both paths share one tokenizer and parser for every per-segment decision, and their whole `Result` — segments with spans, or error code, offset and payload — is compared under every chunking and configuration. The decoding pair (`from_bytes_decoded*`, `from_reader_decoded*`) reports an undefined repertoire byte (`E003` at its raw offset), every fault before it and every limit identically, the prologue racing `max_input_bytes` included. | high | mitigate | mitigated | parse_path_parity.rs::d1_doubled_terminator_is_e002_on_both_paths, ::d4_truncated_una_is_e001_on_both_paths, ::d3a_tag_cut_by_eof_is_e001_on_both_paths, ::a_una_that_is_not_at_the_head_is_e058, ::exhaustive_short_inputs_agree, ::undefined_byte_is_e003_at_its_raw_offset_on_both_paths, ::a_long_prologue_reports_the_first_limit_on_both_paths, ::a_fault_after_a_valid_segment_follows_it_on_both_paths; charset.rs::the_decoding_reader_reports_an_undefined_byte_as_e003; bolero_harness.rs::fuzz_parity_raw, ::fuzz_parity_structured, ::fuzz_reader_and_slice_agree_on_limit_errors; boundary_conditions.rs::slice_and_reader_paths_agree_at_the_limit_boundary; repetition.rs::the_reader_path_agrees_with_the_slice_path; bolero_harness.rs::fuzz_header_read_paths_agree; interchange_header.rs::the_returned_reader_replays_everything |
| E-04 | Silent data loss | Bytes are skipped, replaced or truncated and the result still looks valid: doubled terminators, invalid UTF-8, a dangling release character, an undefined byte in the declared repertoire. | high | mitigate | mitigated | boundary_conditions.rs::consecutive_terminators_are_rejected_not_silently_skipped, ::invalid_utf8_is_rejected_not_silently_replaced, ::dangling_release_at_end_of_input_is_an_error, ::truncated_una_is_an_unexpected_eof, ::empty_elements_and_components_are_preserved; charset.rs::an_undefined_byte_in_a_repertoire_is_an_error_not_a_replacement_character |
| E-05 | Character-set confusion | The declared repertoire is not the one applied: a header that lies about its body, a stateful or multibyte repertoire decoded as single-byte, or a prologue (BOM, leading whitespace) that hides `UNA`/`UNB` from the charset sniffer. The sniffer skips the parser's prologue rule, keeps the prologue untranscoded, and refuses a header outside its fixed window (`E036`) on slice and reader paths alike. | medium | mitigate | mitigated | charset.rs::a_header_that_lies_about_the_body_is_refused, ::stateful_and_multibyte_repertoires_are_refused_rather_than_mis_decoded, ::a_character_outside_the_repertoire_reports_its_offset, ::the_validator_reports_values_outside_the_declared_repertoire, ::the_decoding_reader_matches_whole_slice_decoding, ::prologue_variants_decode_like_the_bare_input, ::prefixed_unsupported_repertoires_are_refused, ::sniff_window_is_measured_after_the_prologue, ::an_over_long_prologue_is_refused_not_passed_through, ::degenerate_prologues_pass_through; parse_path_parity.rs::derived_decoded_pair_agrees |
| E-06 | Structure injection on write | A value containing `'`, `+`, `:`, `*` or `?` ends a segment, starts an element or forges a repetition in the written interchange. Every value is release-escaped against the active service characters; component boundaries are structural, never split from text; trailing empty elements and components are never written. Wrapped message text must be exactly one `UNH`…`UNT` under the builder's service characters, and a raw `UNB`/`UNZ` inside an open interchange is refused, so the writer's `UNZ` count cannot be forged. | high | mitigate | mitigated | bolero_harness.rs::fuzz_escape_value_no_unescaped_delimiters, ::fuzz_anything_the_writer_accepts_reparses, ::fuzz_custom_una_round_trip_is_stable; boundary_conditions.rs::release_escaping_every_service_character_round_trips; repetition.rs::a_value_containing_the_separator_survives_a_round_trip, ::a_rejected_repetition_writes_no_bytes_at_all; charset.rs::escaping_survives_encoding; writer_truncation.rs::ser_drops_an_absent_last_field, ::interior_empties_keep_their_separators; bolero_harness.rs::fuzz_interchange_builder_text_reparses, ::fuzz_writer_owns_the_interchange_count, ::fuzz_message_builder_renders_to_its_segments; interchange_builder.rs::text_that_is_not_one_clean_message_is_refused, ::raw_unb_and_unz_are_refused_inside_an_open_interchange |
| E-07 | Forged trailer | An error path produces a well-formed artefact: an aborted message closed with a valid `UNT`, or segments spliced after a failed write. A `MessageWriter` dropped without `finish()` writes nothing and poisons the writer; after any failure every writer and emitter call returns `WriterPoisoned` and writes nothing; `WriterEmitter::finish` refuses an open segment. | high | mitigate | mitigated | writer_honesty.rs::abort_on_caller_error, ::abort_on_failing_sink, ::abort_on_charset_error_mid_value, ::abort_on_panic, ::a_failed_finish_is_not_retried_on_drop, ::poison_after_sink_failure_blocks_every_method, ::poison_blocks_a_message_writer_obtained_before_the_failure, ::every_failure_index_is_loud, ::refusal_does_not_poison, ::golden_matrix_is_byte_identical; bolero_harness.rs::fuzz_writer_error_paths_never_forge |
| E-08 | Control count and reference mismatch | A `UNT`, `UNE` or `UNZ` count or reference that does not match the content is accepted, so a message is truncated or injected without detection. Each count is compared at its own level (`E005`, `E054`, `E004`, all with the trailer's span); a count that is no unsigned integer is `E048` and never compared silently; an empty group is `E061`; anything after the trailer `UNZ` is `E058`. | high | mitigate | mitigated | envelope_conformance.rs::a_wrong_group_count_is_e054_on_the_une, ::a_wrong_interchange_count_is_e004_on_the_unz, ::a_non_numeric_count_is_e048_only, ::a_signed_or_decimal_count_is_e048_as_well, ::a_huge_count_is_too_long_and_not_compared, ::an_empty_group_is_e061_on_the_ung, ::a_second_interchange_is_e058_only, ::a_una_starting_the_second_interchange_is_e058_on_every_path, ::a_contrl_reports_the_group_count_on_the_ucf; contrl_locations.rs::e054_a_group_count_mismatch_is_29_on_the_ucf_naming_une; contrl_options.rs::bdew_unt_count_rejection_is_byte_exact, ::a_unz_reference_mismatch_names_unz_on_the_uci, ::a_une_reference_mismatch_names_une_on_the_ucf, ::a_duplicate_unh_reference_names_unh_on_the_ucm; recovering_scan.rs::utilmd_interchange_count_mismatch_is_29_on_the_ucm_naming_unt |
| E-09 | Unchecked service-segment values | A 15-character control reference, an impossible date, an unknown syntax version, a version 3 field in its version 4 length, or an interactive, security or reserved `U` segment passes envelope validation, and downstream code that trusts the envelope stores or routes on it. Every envelope entry point checks every `UNB`/`UNZ`/`UNG`/`UNE`/`UNH`/`UNT` position against the ISO 9735-10 table for the declared syntax version (1–4, else `E059`), S004 as a calendar date and time (`E060`), and refuses `U` tags by name (`E055`–`E057`), all as errors; the table is pinned to a transcription of the standard, and the envelope and `DirectoryValidator(service::lookup)` share one check. | medium | mitigate | mitigated | envelope_conformance.rs::a_v3_date_of_eight_digits_is_too_long, ::a_v4_date_of_six_digits_is_too_short, ::a_control_reference_of_15_characters_is_too_long_in_unb_and_unz, ::de_0008_is_an_14_in_v3_and_an_35_in_v4, ::an_impossible_date_and_time_are_e060, ::an_unsupported_syntax_version_is_e059, ::every_refused_tag_fails_validate_envelope, ::envelope_and_directory_validator_report_the_same_service_findings; service_conformance.rs::every_batch_service_position_matches_the_transcription; validation.rs::envelope_dedupe::spanless_findings_are_never_dropped, ::envelope_dedupe::a_spanless_finding_both_layers_make_is_reported_once; bolero_harness.rs::fuzz_service_segments |
| E-10 | False or misaddressed acknowledgement | A `CONTRL` acknowledges an interchange that did not parse, invents a trailer, or is addressed without the partner's qualifiers and is therefore lost or misrouted. The reply `UNB` is a validated `InterchangeHeader` written by `Writer::begin_interchange`, so an empty, over-long or unencodable party is a typed error with no bytes, and identity values are escaped once. | high | mitigate | mitigated | contrl_options.rs::a_terminal_fault_never_acknowledges, ::a_receipt_under_a_7_4_policy_is_refused, ::ucm_copies_the_subject_association_code, ::no_readable_addressing_is_no_contrl, ::a_fault_in_an_unnamed_message_rejects_the_uci, ::reply_unb_keeps_party_qualifiers, ::a_policy_that_forbids_rejecting_the_interchange_is_loud, ::reply_with_an_empty_party_is_refused, ::reply_with_an_over_long_party_is_refused, ::replies_pass_validate_envelope, ::reply_identity_values_round_trip_through_escaping; recovering_scan.rs::a_message_without_unt_gets_no_invented_trailer, ::a_terminal_fault_leaves_the_interchange_incomplete; contrl_locations.rs::every_location_from_fuzzed_input_satisfies_the_invariants; bolero_harness.rs::fuzz_contrl_options_reparse |
| E-11 | Log injection through error text | A value from the input (reference, qualifier, tag) carries CR/LF, ESC, U+2028 or a bidirectional override into error text, forging extra log lines, reordering one or driving a terminal. `Display` of `EdifactError`, `ValidationIssue`, `ValidationReport`, `GroupPath` and the envelope summaries, `render_deterministic` and the `miette` help and labels write every control character, U+2028/U+2029 and the bidirectional formatting characters (Unicode `Bidi_Control`) as an escape (`\r`, `\n`, `\u{1b}`, `\u{202e}`); the fields keep the raw values. An in-crate test without a wildcard arm (`error::tests::display_escapes_every_variant`) fails to compile until a new variant is covered. | medium | mitigate | mitigated | error_text.rs::forged_reference_prints_on_one_line, ::clean_values_print_unchanged, ::report_renders_one_line_per_issue, ::escaping_is_idempotent, ::miette_help_and_labels_are_escaped, ::bidi_overrides_are_escaped; bolero_harness.rs::fuzz_error_text_has_no_control_chars |
| E-12 | Lenient typed deserialization | A typed (derive) model is filled from incomplete data: an absent required element or component becomes an empty string or a default, or an optional field swallows a segment error. | medium | mitigate | mitigated | derive_deserialize_strict.rs::required_option_fails_when_element_absent, ::required_option_fails_when_element_empty_string, ::required_component_fails_with_missing_required_component_when_component_empty, ::non_optional_segment_field_requires_non_empty_value, ::optional_message_field_does_not_swallow_segment_errors |
| E-13 | Misplaced findings | A finding's location (segment occurrence, element, component) overflows or saturates on large input, so an acknowledgement or report points at the wrong segment. Fields are `u32`/`u16`, and a value that does not fit is `None`, never clamped. Segment positions count from each message's own `UNH`, in every layer, envelope findings included. | low | mitigate | mitigated | validation.rs::seventy_thousand_segment_occurrence_is_exact, ::every_builtin_issue_has_layer_and_kind, ::positions_count_from_each_messages_unh |
| E-14 | Algorithmic complexity in group resolution | Deep or wide segment-group trees, or many sibling groups, make placement quadratic or worse. Ordered resolution and direct indexing are linear in the segment count. In-crate tests count the placement steps (`group::tests::placement_work_is_linear`: at most 2.2× the work when the sibling groups double) and refuse invalid schemas (`group::tests::invalid_group_schemas_are_refused`). | medium | mitigate | mitigated | bolero_harness.rs::fuzz_group_ordered_invariants, ::fuzz_group_open_matches_legacy_reference; benchmark group `grouping` (baseline) |
| E-15 | Supply chain and release | A compromised dependency or release pipeline ships altered code, or a release breaks the API unannounced. There are three required dependencies, `cargo deny` runs in CI, and crates.io releases go through Trusted Publishing (no registry token). CI dry-run publishes both crates, checks each package for README and licence texts, and runs cargo-semver-checks against the last release (read-only token, SHA-pinned installer). | medium | mitigate | partial | CI jobs `release-check` (`scripts/package-contents-check.sh`) and `semver` (`.github/workflows/ci.yml`); `deny.toml` |
| E-16 | Date/time misreading | A `DTM` value is half-accepted (wrong width, non-ASCII digit, hour 24 with minutes, zone `+15` or `+0530`, count `01`, hyphenated period) or panics the decoder, or a comparison the codec cannot decide (floating vs zoned within 28 h, inverted period, count, a `104` week against a date) is read as ordered. Feature `dtm` decodes only exact layouts, formats every accepted value back byte for byte, uses no implicit offset, and returns `Indeterminate` for every undecided relation; `DtmValidator` reports a value it could not check because its code is unsupported. | medium | mitigate, harness | mitigated | dtm_codec.rs::invalid_values_are_typed_errors_with_position, ::non_ascii_at_every_width_is_refused_without_panic, ::round_trip_every_code; dtm_compare.rs::deny_by_default_rejects_every_undecided_pair, ::range_edge_relations, ::range_edges_do_not_panic_at_any_offset, ::code_104_compares_only_with_104; dtm_validator.rs::unsupported_code_is_a_warning_on_the_code; bolero_harness.rs::fuzz_dtm_decode_round_trip |

### Harness responsibilities

- **E-01** The defaults are bounded and generous; tighten them to your
  traffic. Stream untrusted input (`from_reader`, `message_windows*`,
  `deserialize_messages*`): collecting a whole interchange
  (`from_bytes(..).collect()`, `deserialize`, `from_bytes_decoded`,
  `RecoveredInterchange::scan`) takes up to 128 × its bytes in heap, at most
  128 × `max_input_bytes`. Use `ReaderConfig::unbounded()` only for trusted
  input. Each entry point's rustdoc has a `# Resource bounds` section.
- **Values are data.** Values returned by the parser are untrusted text in
  the declared repertoire. Encode them for wherever they go next (SQL, HTML,
  file names, logs).
- **E-16** Accept a period only on `DtmRelation::Before`; `Indeterminate`
  and `Overlapping` are refusals. A gate that admits only checked values also
  refuses `DtmValidator`'s unsupported-code warning.
- **Authenticity is the transport's job.** A well-formed interchange proves
  nothing about who sent it.

### Open variants

- **E-02** The fuzz targets run as a smoke test (50 000 iterations;
  1 000 000 for `fuzz_parity_*`) with no persistent corpus and no scheduled
  long run.
- **E-15** Released crates carry no build attestation. The semver gate does
  not analyse `edifact-derive`; its UI suite covers the attribute grammar.
