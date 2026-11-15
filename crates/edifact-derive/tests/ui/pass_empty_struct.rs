#[path = "support.rs"]
mod support;

pub use support::edifact::{
    Element, EdifactDeserialize, EdifactError, EdifactEvent, EdifactSegmentTag,
    EdifactSerialize, EventEmitter, Segment, find_qualified_segment, find_segment};

extern crate self as edifact;
pub use support::edifact::helpers;

use edifact_derive::{EdifactDeserialize as DeriveEdifactDeserialize, EdifactSerialize as DeriveEdifactSerialize};

#[derive(DeriveEdifactSerialize, DeriveEdifactDeserialize)]
struct Empty {}

fn main() {
    let _ = std::any::type_name::<Empty>();
}