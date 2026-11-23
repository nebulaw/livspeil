// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Capability tokens granted to a script by its host.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// One capability token. Its text form is what the handshake reports and
/// what `livspeil run --grant` takes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Capability {
    /// `graph.read`: list nodes, read inputs, outputs and links, take
    /// snapshots, and receive scene `event`s.
    GraphRead,
    /// `graph.write`: apply commands through the command bus (undoable,
    /// journaled with origin `script`) and allocate node ids.
    GraphWrite,
    /// `graph.cook`: cook nodes and read their output values.
    GraphCook,
    /// `fs.read:<root>`: nodes that read the file system (such as
    /// `core.fs.list_dir`) see files under `<root>` when the script cooks
    /// them. Without it they have no file system.
    FsRead(PathBuf),
}

impl Capability {
    /// The token's text form.
    #[must_use]
    pub fn token(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::GraphRead => f.write_str("graph.read"),
            Capability::GraphWrite => f.write_str("graph.write"),
            Capability::GraphCook => f.write_str("graph.cook"),
            Capability::FsRead(root) => write!(f, "fs.read:{}", root.display()),
        }
    }
}

/// A capability token that is not known or malformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityParseError {
    /// The rejected token.
    pub token: String,
}

impl fmt::Display for CapabilityParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown capability '{}' (known: graph.read, graph.write, graph.cook, fs.read:<root>)",
            self.token
        )
    }
}

impl std::error::Error for CapabilityParseError {}

impl FromStr for Capability {
    type Err = CapabilityParseError;

    fn from_str(token: &str) -> Result<Self, Self::Err> {
        match token {
            "graph.read" => Ok(Capability::GraphRead),
            "graph.write" => Ok(Capability::GraphWrite),
            "graph.cook" => Ok(Capability::GraphCook),
            _ => match token.strip_prefix("fs.read:") {
                Some(root) if !root.is_empty() => Ok(Capability::FsRead(PathBuf::from(root))),
                _ => Err(CapabilityParseError {
                    token: token.to_owned(),
                }),
            },
        }
    }
}

/// The set of capabilities granted to one script session. Empty by
/// default: a script with no grants can only say hello, log and listen
/// for ticks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    granted: Vec<Capability>,
}

impl Capabilities {
    /// No capabilities.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Grant `capability` (granting twice is a no-op).
    pub fn grant(&mut self, capability: Capability) -> &mut Self {
        if !self.granted.contains(&capability) {
            self.granted.push(capability);
            self.granted.sort();
        }
        self
    }

    /// Builder form of [`Capabilities::grant`].
    #[must_use]
    pub fn with(mut self, capability: Capability) -> Self {
        self.grant(capability);
        self
    }

    /// Parse tokens such as `["graph.read", "fs.read:/tmp"]`.
    ///
    /// # Errors
    /// The first unknown or malformed token.
    pub fn parse<'a>(
        tokens: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, CapabilityParseError> {
        let mut caps = Capabilities::none();
        for token in tokens {
            caps.grant(token.parse()?);
        }
        Ok(caps)
    }

    /// Whether `capability` is granted.
    #[must_use]
    pub fn has(&self, capability: &Capability) -> bool {
        self.granted.contains(capability)
    }

    /// The root of the first `fs.read` grant, if any.
    #[must_use]
    pub fn fs_root(&self) -> Option<&PathBuf> {
        self.granted.iter().find_map(|c| match c {
            Capability::FsRead(root) => Some(root),
            _ => None,
        })
    }

    /// The granted capabilities, sorted.
    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.granted.iter()
    }

    /// The granted tokens, sorted.
    #[must_use]
    pub fn tokens(&self) -> Vec<String> {
        self.granted.iter().map(Capability::token).collect()
    }

    /// Whether nothing is granted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.granted.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display_round_trip() {
        for token in [
            "graph.read",
            "graph.write",
            "graph.cook",
            "fs.read:/srv/data",
        ] {
            let cap: Capability = token.parse().unwrap();
            assert_eq!(cap.to_string(), token);
        }
        assert!("graph.delete".parse::<Capability>().is_err());
        assert!("fs.read:".parse::<Capability>().is_err());
    }

    #[test]
    fn set_is_sorted_and_deduplicated() {
        let caps = Capabilities::parse(["graph.write", "graph.read", "graph.write"]).unwrap();
        assert_eq!(caps.tokens(), ["graph.read", "graph.write"]);
        assert!(caps.has(&Capability::GraphRead));
        assert!(!caps.has(&Capability::GraphCook));
    }
}
