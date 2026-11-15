#[path = "support.rs"]
mod support;

pub use support::edifact::{
    CompositeElement, Element, EdifactCompositeDeserialize, EdifactCompositeSerialize,
    EdifactDeserialize, EdifactError, EdifactEvent, EdifactSegmentTag, EdifactSerialize,
    EventEmitter, Segment, composite_element, SegmentSliceExt};

extern crate self as edifact;
pub use support::edifact::helpers;

use edifact_derive::{
    EdifactDeserialize as DeriveEdifactDeserialize,
    EdifactSerialize as DeriveEdifactSerialize};

#[derive(DeriveEdifactSerialize, DeriveEdifactDeserialize)]
#[edifact(segment = "RFF")]
struct RffSegment {
    #[edifact(element = 0, composite)]
    reference: Vec<String>}

fn main() {
    let _ = std::any::type_name::<RffSegment>();
}