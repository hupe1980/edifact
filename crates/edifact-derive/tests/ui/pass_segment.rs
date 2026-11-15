#[path = "support.rs"]
mod support;

pub use support::edifact::{
    Element, EdifactDeserialize, EdifactError, EdifactEvent, EdifactSegmentTag,
    EdifactSerialize, EventEmitter, Segment, SegmentSliceExt};

extern crate self as edifact;
pub use support::edifact::helpers;

use edifact_derive::{
    EdifactDeserialize as DeriveEdifactDeserialize,
    EdifactSerialize as DeriveEdifactSerialize};

#[derive(DeriveEdifactSerialize, DeriveEdifactDeserialize)]
#[edifact(segment = "BGM")]
struct BgmSegment {
    #[edifact(element = 0)]
    doc_name_code: String,
    #[edifact(element = 1)]
    doc_id: String,
    #[edifact(element = 2)]
    msg_function: Option<String>}

fn main() {
    let _ = std::any::type_name::<BgmSegment>();
}