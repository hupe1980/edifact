//! The envelope layer and `DirectoryValidator(service::lookup)` report the
//! same presence, class and length findings on the service segments.

use edifact::{
    DirectoryValidator, OwnedSegment, Span, ValidationContext, ValidationIssue, ValidationLayer,
    from_bytes, service, validate_envelope_lenient,
};

fn envelope_layer(segments: &[OwnedSegment]) -> Vec<ValidationIssue> {
    ValidationContext::builder()
        .with_envelope_validation()
        .structure(false)
        .code_list(false)
        .profile(false)
        .build()
        .validate(segments)
        .errors()
        .to_vec()
}

pub fn service_directory() -> DirectoryValidator {
    DirectoryValidator::new(
        "iso-9735-service",
        service::lookup,
        |_, _| true,
        |_, _| None,
        |_, _| None,
        None,
    )
}

type Finding = (
    String,
    Option<String>,
    Option<u16>,
    Option<u16>,
    Option<Span>,
);

/// The presence/class/length findings on the six envelope segments.
fn representation_findings(issues: &[ValidationIssue]) -> Vec<Finding> {
    let mut out: Vec<Finding> = issues
        .iter()
        .filter(|i| {
            matches!(
                i.error_code(),
                Some("E008" | "E021" | "E048" | "E049" | "E050")
            ) && matches!(
                i.segment_tag(),
                Some("UNB" | "UNZ" | "UNG" | "UNE" | "UNH" | "UNT")
            )
        })
        .map(|i| {
            (
                i.error_code().unwrap_or_default().to_owned(),
                i.segment_tag().map(str::to_owned),
                i.element_index,
                i.component_index,
                i.span,
            )
        })
        .collect();
    out.sort_by_key(|f| {
        (
            f.0.clone(),
            f.1.clone(),
            f.2,
            f.3,
            f.4.map(|s| (s.start, s.end)),
        )
    });
    out
}

/// Assert that the envelope layer and `DirectoryValidator(service::lookup)`
/// report the same presence, class and length findings on `input`.
pub fn assert_same_service_findings(input: &[u8]) {
    let Ok(segments) = from_bytes(input)
        .map(|r| r.map(|s| s.into_owned()))
        .collect::<Result<Vec<_>, _>>()
    else {
        return;
    };
    let envelope = representation_findings(&envelope_layer(&segments));
    let directory = representation_findings(
        ValidationContext::builder()
            .with_validator(ValidationLayer::Structure, service_directory())
            .build()
            .validate(&segments)
            .errors(),
    );
    // The envelope stops at structural faults it cannot interpret, so compare
    // only when it got as far as the trailer.
    if validate_envelope_lenient(&segments).interchange.is_some() {
        assert_eq!(envelope, directory, "{}", String::from_utf8_lossy(input));
    }
}
