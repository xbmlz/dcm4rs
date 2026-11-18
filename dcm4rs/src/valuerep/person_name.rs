//! VR `PN`: a person name with up to three component groups.

use std::fmt;

use crate::error::{Error, Result};

/// Longest component PS3.5 6.2 allows, in characters.
const MAX_COMPONENT: usize = 64;

/// A person name: the alphabetic, ideographic and phonetic groups, in that order.
///
/// A group holds up to five `^`-separated components — family name, given name, middle name,
/// name prefix and name suffix — and the groups themselves are `=`-separated. The value kept
/// here is the decoded text, so the ideographic and phonetic groups are already in characters
/// rather than the ISO 2022 escape sequences the file carried.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PersonName {
    groups: Vec<String>,
}

impl PersonName {
    /// Parse a decoded `PN` value.
    ///
    /// Trailing empty groups are dropped, as pydicom does, so `Doe^John=` and `Doe^John` are the
    /// same name. A name may legitimately have an empty alphabetic group with an ideographic one
    /// (`=山田^太郎`), which keeps the leading delimiter.
    pub fn parse(text: &str) -> Result<Self> {
        let mut groups: Vec<&str> = text.split('=').collect();
        while groups.last().is_some_and(|group| group.is_empty()) {
            groups.pop();
        }
        for group in &groups {
            validate_group(group, false)?;
        }
        Ok(Self {
            groups: groups.into_iter().map(str::to_owned).collect(),
        })
    }

    /// Build a name from the five alphabetic components.
    ///
    /// Trailing empty components are dropped, so an all-empty call produces an empty name.
    pub fn from_named_components(
        family: &str,
        given: &str,
        middle: &str,
        prefix: &str,
        suffix: &str,
    ) -> Result<Self> {
        let mut name = Self::default();
        name.push_group(&[family, given, middle, prefix, suffix])?;
        Ok(name)
    }

    /// Append a component group, in the order the group appears in the value.
    ///
    /// Used to add the ideographic and phonetic groups to a name built from components.
    pub fn with_group(mut self, components: &[&str]) -> Result<Self> {
        self.push_group(components)?;
        Ok(self)
    }

    /// The groups, most significant first.
    pub fn groups(&self) -> &[String] {
        &self.groups
    }

    /// The alphabetic group, empty when the name has none.
    pub fn alphabetic(&self) -> &str {
        self.groups.first().map_or("", String::as_str)
    }

    /// The ideographic group, empty when the name has none.
    pub fn ideographic(&self) -> &str {
        self.groups.get(1).map_or("", String::as_str)
    }

    /// The phonetic group, empty when the name has none.
    pub fn phonetic(&self) -> &str {
        self.groups.get(2).map_or("", String::as_str)
    }

    /// The `index`th component of the `group`th group, empty when it is absent.
    pub fn component(&self, group: usize, index: usize) -> &str {
        self.groups
            .get(group)
            .and_then(|text| text.split('^').nth(index))
            .unwrap_or("")
    }

    /// The family name, the alphabetic group's first component.
    pub fn family_name(&self) -> &str {
        self.component(0, 0)
    }

    /// The given name, the alphabetic group's second component.
    pub fn given_name(&self) -> &str {
        self.component(0, 1)
    }

    /// The middle name, the alphabetic group's third component.
    pub fn middle_name(&self) -> &str {
        self.component(0, 2)
    }

    /// The name prefix, the alphabetic group's fourth component.
    pub fn name_prefix(&self) -> &str {
        self.component(0, 3)
    }

    /// The name suffix, the alphabetic group's fifth component.
    pub fn name_suffix(&self) -> &str {
        self.component(0, 4)
    }

    /// Whether the name has no non-empty group.
    pub fn is_empty(&self) -> bool {
        self.groups.iter().all(|group| group.is_empty())
    }

    /// The value as `PN` text: groups joined by `=`, components by `^`.
    pub fn to_value(&self) -> String {
        self.groups.join("=")
    }

    /// Append one group, dropping its trailing empty components.
    fn push_group(&mut self, components: &[&str]) -> Result<()> {
        let mut parts: Vec<&str> = components.to_vec();
        while parts.last().is_some_and(|part| part.is_empty()) {
            parts.pop();
        }
        for part in &parts {
            validate_group(part, true)?;
        }
        self.groups.push(parts.join("^"));
        Ok(())
    }
}

/// Reject a group that cannot be stored: a component longer than PS3.5 allows, or a character
/// that would read back as a delimiter.
///
/// Parsing tolerates `^` and `=` because they are the delimiters it just split on, and it only
/// rejects the backslash, which separates one person name from the next. Building a value from
/// components rejects all three.
fn validate_group(group: &str, from_components: bool) -> Result<()> {
    if group.contains('\\') {
        return Err(Error::InvalidValue(format!(
            "a person name cannot contain a backslash: {group}"
        )));
    }
    if from_components {
        for delimiter in ['^', '='] {
            if group.contains(delimiter) {
                return Err(Error::InvalidValue(format!(
                    "a person name component cannot contain '{delimiter}': {group}"
                )));
            }
        }
    }
    for component in group.split('^') {
        if component.chars().count() > MAX_COMPONENT {
            return Err(Error::InvalidValue(format!(
                "a person name component exceeds {MAX_COMPONENT} characters: {component}"
            )));
        }
    }
    Ok(())
}

impl fmt::Display for PersonName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_value())
    }
}

impl std::str::FromStr for PersonName {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::parse(text)
    }
}
