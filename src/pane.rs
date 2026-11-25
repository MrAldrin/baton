//! Terminal pane ids.

use std::fmt;
use std::str::FromStr;

use anyhow::{Context, bail};

/// A terminal pane id. Parses `terminal_N` or `N`; prints `terminal_N`.
///
/// ```
/// use baton::pane::PaneId;
/// assert_eq!("terminal_3".parse::<PaneId>().unwrap(), PaneId(3));
/// assert_eq!("3".parse::<PaneId>().unwrap().to_string(), "terminal_3");
/// assert!("plugin_2".parse::<PaneId>().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneId(pub u32);

impl FromStr for PaneId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let digits = s.strip_prefix("terminal_").unwrap_or(s);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            bail!("invalid pane id {s:?}: use terminal_N or N");
        }
        let n = digits
            .parse()
            .with_context(|| format!("invalid pane id {s:?}"))?;
        Ok(Self(n))
    }
}

impl fmt::Display for PaneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "terminal_{}", self.0)
    }
}

impl serde::Serialize for PaneId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for PaneId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}
