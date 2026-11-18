//! Private creator blocks (PS3.5, 7.8.1).
//!
//! A private creator claims one block of a private group: `(gggg,0010)` through `(gggg,00FF)`
//! name the creators that own the group, and the block's elements live at
//! `(gggg,1000 + block * 0x100 + offset)` where `offset` is the low byte the creator defined.
//! This mirrors pydicom's `DataSet.private_block`. Which VR an offset carries still comes from
//! [`PrivateDictionary`](crate::PrivateDictionary), which is where vendor registries go.

use crate::dataset::DataSet;
use crate::error::{Error, Result};
use crate::tag::Tag;
use crate::vr::Vr;

/// The first creator slot, `(gggg,0010)`.
const FIRST_CREATOR_ELEMENT: u16 = 0x0010;
/// The last creator slot, `(gggg,00FF)`.
const LAST_CREATOR_ELEMENT: u16 = 0x00FF;
/// Where the first block's elements begin, `(gggg,1000)`.
const FIRST_BLOCK_ELEMENT: u16 = 0x1000;

/// The elements one private creator owns in one group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivateBlock {
    group: u16,
    block: u16,
}

impl PrivateBlock {
    /// The group the block belongs to.
    pub const fn group(self) -> u16 {
        self.group
    }

    /// The block number, the creator slot's element offset from `(gggg,0010)`.
    pub const fn block(self) -> u16 {
        self.block
    }

    /// The tag of the element at `offset` within this block, `(gggg,1000 + block * 0x100 + offset)`.
    pub const fn tag(self, offset: u16) -> Tag {
        Tag::new(
            self.group,
            FIRST_BLOCK_ELEMENT + self.block * 0x100 + (offset & 0x00FF),
        )
    }
}

impl DataSet {
    /// Find the block `creator` owns in `group`.
    ///
    /// `None` also covers a group that is not private: PS3.5 7.8.1 gives private groups an odd
    /// group number, and this does not guess a block for an even one.
    pub fn private_block(&self, group: u16, creator: &str) -> Option<PrivateBlock> {
        validate_group(group).ok()?;
        let wanted = normalize(creator);
        for element in FIRST_CREATOR_ELEMENT..=LAST_CREATOR_ELEMENT {
            let tag = Tag::new(group, element);
            if let Some(found) = self.get(tag) {
                if let Ok(value) = found.string() {
                    if normalize(value) == wanted {
                        return Some(PrivateBlock {
                            group,
                            block: element - FIRST_CREATOR_ELEMENT,
                        });
                    }
                }
            }
        }
        None
    }

    /// Find the block `creator` owns, reserving the next creator slot when it has none.
    ///
    /// The reserved slot holds `creator` as `LO`, as PS3.5 7.8.1 asks. Elements of the block are
    /// written by the caller at [`PrivateBlock::tag`]; the VR is the caller's, because only a
    /// vendor registry knows it.
    pub fn private_block_or_create(&mut self, group: u16, creator: &str) -> Result<PrivateBlock> {
        validate_group(group)?;
        if let Some(block) = self.private_block(group, creator) {
            return Ok(block);
        }
        for element in FIRST_CREATOR_ELEMENT..=LAST_CREATOR_ELEMENT {
            let tag = Tag::new(group, element);
            if self.get(tag).is_none() {
                self.set_string(tag, Vr::Lo, creator);
                return Ok(PrivateBlock {
                    group,
                    block: element - FIRST_CREATOR_ELEMENT,
                });
            }
        }
        Err(Error::InvalidTag(format!(
            "group {group:04X} has no free private creator slot"
        )))
    }
}

/// Reject a group that cannot be private.
fn validate_group(group: u16) -> Result<()> {
    if group % 2 == 0 {
        return Err(Error::InvalidTag(format!(
            "{group:04X} is not a private group: private groups are odd (PS3.5, 7.8.1)"
        )));
    }
    Ok(())
}

/// Strip the padding a creator name may carry: `LO` pads with a space, some writers with NUL.
fn normalize(value: &str) -> &str {
    value.trim_matches(|ch: char| ch == '\0' || ch.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_block_and_computes_its_tags() {
        let mut set = DataSet::new();
        let block = set
            .private_block_or_create(0x0009, "ACME 1.0")
            .expect("block");
        assert_eq!(block.group(), 0x0009);
        assert_eq!(block.block(), 0);
        assert_eq!(block.tag(0x01), Tag::new(0x0009, 0x1001));
        assert_eq!(block.tag(0xFF), Tag::new(0x0009, 0x10FF));
        assert_eq!(
            set.get(Tag::new(0x0009, 0x0010))
                .expect("creator")
                .string()
                .expect("text"),
            "ACME 1.0"
        );
    }

    #[test]
    fn finds_the_block_it_created() {
        let mut set = DataSet::new();
        let created = set
            .private_block_or_create(0x0009, "ACME 1.0")
            .expect("block");
        set.set_string(created.tag(0x01), Vr::Lo, "value");
        let found = set.private_block(0x0009, "ACME 1.0").expect("block");
        assert_eq!(found, created);
        assert_eq!(
            set.get(found.tag(0x01))
                .expect("element")
                .string()
                .expect("text"),
            "value"
        );
    }

    #[test]
    fn a_second_creator_takes_the_next_slot() {
        let mut set = DataSet::new();
        let first = set
            .private_block_or_create(0x0009, "ACME 1.0")
            .expect("first");
        let second = set
            .private_block_or_create(0x0009, "OTHER")
            .expect("second");
        assert_eq!(first.block(), 0);
        assert_eq!(second.block(), 1);
        assert_eq!(second.tag(0x01), Tag::new(0x0009, 0x1101));
    }

    #[test]
    fn a_creator_name_is_compared_without_its_padding() {
        let mut set = DataSet::new();
        set.set_string(Tag::new(0x0009, 0x0010), Vr::Lo, "ACME 1.0 ");
        let block = set.private_block(0x0009, " ACME 1.0").expect("block");
        assert_eq!(block.block(), 0);
    }

    #[test]
    fn an_even_group_has_no_block() {
        let mut set = DataSet::new();
        assert!(set.private_block(0x0008, "ACME 1.0").is_none());
        assert!(set.private_block_or_create(0x0008, "ACME 1.0").is_err());
    }

    #[test]
    fn an_unknown_creator_is_none() {
        let set = DataSet::new();
        assert!(set.private_block(0x0009, "ACME 1.0").is_none());
    }
}
