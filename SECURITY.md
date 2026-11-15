# Security policy

## Reporting a vulnerability

Please report a suspected vulnerability **privately** through GitHub's
private vulnerability reporting:
<https://github.com/hupe1980/edifact/security/advisories/new>.

Do not open a public issue, pull request or discussion for it. Please
include the `edifact` version, the entry point you called (for example
`from_reader` with a given `ReaderConfig`), and the smallest input that
shows the problem. Attach the input as bytes, not pasted text, because
separators and character sets matter.

## Scope

The [threat model](crates/edifact/THREAT_MODEL.md) describes what `edifact`
defends against and what it leaves out, and lists each known threat class
with its status. Reports about classes it marks out of scope, such as
transport security or business-rule validation, are still welcome. They may
be answered with a pointer to the layer that owns them.

## Supported versions

| Version | Supported |
|---|---|
| 0.20.x | Yes |
| Earlier, including every `edifact-rs` release | No |

Before 1.0, only the latest minor version gets fixes; there are no
backports.
