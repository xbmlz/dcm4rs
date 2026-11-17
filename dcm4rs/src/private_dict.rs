//! Optional registry for private data element VRs.
//!
//! Private tags are resolved from their private creator: `(gggg,00xx)` holds the creator name
//! and `(gggg,xxee)` holds a private element whose VR is defined by the creator, not by the
//! standard. This registry lets callers supply those VRs without bundling a vendor database.

use std::collections::HashMap;

use crate::tag::Tag;
use crate::vr::Vr;

/// A mapping from private creators and element offsets to value representations.
#[derive(Clone, Debug, Default)]
pub struct PrivateDictionary {
    entries: HashMap<(String, u16, u16), Vr>,
}

impl PrivateDictionary {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the VR of a private element.
    ///
    /// `tag` supplies the group and the element offset; only the low byte of the element is
    /// stored because the block byte (`xx` in `(gggg,xxee)`) varies between files.
    pub fn insert(&mut self, creator: impl Into<String>, tag: Tag, vr: Vr) -> Option<Vr> {
        self.entries
            .insert((creator.into(), tag.group(), tag.element() & 0x00FF), vr)
    }

    /// VR registered for `creator` at `group` and `element_offset`, if any.
    pub fn get(&self, creator: &str, group: u16, element_offset: u16) -> Option<Vr> {
        self.entries
            .get(&(creator.to_owned(), group, element_offset & 0x00FF))
            .copied()
    }

    /// Number of registered entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
