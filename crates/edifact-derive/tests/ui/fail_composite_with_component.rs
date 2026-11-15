#[path = "support.rs"]
mod support;

pub use support::edifact::{
    Element, EdifactCompositeDeserialize, EdifactCompositeSerialize, EdifactDeserialize,
    EdifactError, EdifactEvent, EdifactSegmentTag, EdifactSerialize, EventEmitter, Segment,
    composite_element, SegmentSliceExt};

extern crate self as edifact;

use edifact_derive::EdifactSerialize as DeriveEdifactSerialize;

#[derive(DeriveEdifactSerialize)]
#[edifact(segment = "RFF")]
struct RffSegment {
    #[edifact(element = 0, component = 1, composite)]
    reference: Vec<String>}

fn main() {}