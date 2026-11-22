// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Public node and Charm API surface.
//!
//! This crate is the part of Livspeil that node and Charm authors compile
//! against. It is licensed under the LGPL (decision D25); the rest of the
//! kernel is GPL. See `specs/livspeil-kernel-spec-v0.md`, sections 2 and 4.

/// Value types that can flow through node ports (kernel spec, section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum PortType {
    Float,
    Int,
    Bool,
    Vec2,
    Vec3,
    Vec4,
    /// OKLCH plus alpha.
    Color,
    String,
    Enum,
    Event,
    Table,
    Text,
    /// Vector path.
    Path,
    Image,
    Geometry,
    Material,
    Scene,
    Window,
}

impl PortType {
    /// Every port type known to this version of the API.
    pub const ALL: [PortType; 18] = [
        PortType::Float,
        PortType::Int,
        PortType::Bool,
        PortType::Vec2,
        PortType::Vec3,
        PortType::Vec4,
        PortType::Color,
        PortType::String,
        PortType::Enum,
        PortType::Event,
        PortType::Table,
        PortType::Text,
        PortType::Path,
        PortType::Image,
        PortType::Geometry,
        PortType::Material,
        PortType::Scene,
        PortType::Window,
    ];

    /// Stable name used in documents and node specs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            PortType::Float => "Float",
            PortType::Int => "Int",
            PortType::Bool => "Bool",
            PortType::Vec2 => "Vec2",
            PortType::Vec3 => "Vec3",
            PortType::Vec4 => "Vec4",
            PortType::Color => "Color",
            PortType::String => "String",
            PortType::Enum => "Enum",
            PortType::Event => "Event",
            PortType::Table => "Table",
            PortType::Text => "Text",
            PortType::Path => "Path",
            PortType::Image => "Image",
            PortType::Geometry => "Geometry",
            PortType::Material => "Material",
            PortType::Scene => "Scene",
            PortType::Window => "Window",
        }
    }
}

/// Static description of a node type (kernel spec, section 9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSpec {
    /// Namespaced id, for example `core.math.lerp`.
    pub id: &'static str,
    /// Spec version; bumped on breaking changes to ports or behavior.
    pub version: u32,
    /// One-line description.
    pub summary: &'static str,
    /// Whether the node must be re-cooked on every frame tick.
    pub time_dependent: bool,
}

impl NodeSpec {
    /// Id with version, for example `core.math.lerp@1`.
    #[must_use]
    pub fn qualified_id(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn port_type_names_are_unique_and_non_empty() {
        let names: HashSet<&str> = PortType::ALL.iter().map(|t| t.name()).collect();
        assert_eq!(names.len(), PortType::ALL.len());
        assert!(names.iter().all(|n| !n.is_empty()));
    }

    #[test]
    fn all_lists_every_variant_once() {
        let unique: HashSet<PortType> = PortType::ALL.iter().copied().collect();
        assert_eq!(unique.len(), PortType::ALL.len());
    }

    #[test]
    fn qualified_id_includes_version() {
        let spec = NodeSpec {
            id: "core.math.lerp",
            version: 1,
            summary: "Linear interpolation",
            time_dependent: false,
        };
        assert_eq!(spec.qualified_id(), "core.math.lerp@1");
    }
}
