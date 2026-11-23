// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reading shared Charm definitions (M0.16): the `charms` block and the
//! `def="id@version"` property of instance nodes. See the crate
//! documentation for the syntax.

use std::collections::{BTreeMap, BTreeSet};

use kdl::KdlNode;
use livspeil_graph::{CHARM_TYPE, CharmMeta};

use super::{Entries, Reader, children};
use crate::error::DocError;
use crate::model::{DocCharmDef, DocDefRef, DocNode, Network, SourceLoc, Tier};

impl Reader<'_> {
    /// `charms { def ... }`.
    pub(super) fn charms(&self, node: &KdlNode) -> Result<Vec<DocCharmDef>, DocError> {
        let entries = self.entries(node)?;
        self.allow_props(&entries, &[])?;
        if let Some(arg) = entries.args.first() {
            return Err(self.err_entry(arg, "`charms` takes no arguments"));
        }
        let mut defs: Vec<DocCharmDef> = Vec::new();
        let mut ids = BTreeSet::new();
        for child in children(node) {
            self.no_annotation(child)?;
            if child.name().value() != "def" {
                return Err(self.err_node(
                    child,
                    format!(
                        "unknown node `{}` in charms; expected `def \"id\" version=\"..\" {{ ... }}`",
                        child.name().value()
                    ),
                ));
            }
            let def = self.def(child)?;
            if !ids.insert(def.id.clone()) {
                return Err(
                    self.err_node(child, format!("duplicate Charm definition '{}'", def.id))
                );
            }
            defs.push(def);
        }
        Ok(defs)
    }

    /// `def "id" version=".." [name=".."] [icon=".."] [min-tier=".."] {
    /// capability ".."  network { ... } }`.
    fn def(&self, node: &KdlNode) -> Result<DocCharmDef, DocError> {
        let entries = self.entries(node)?;
        self.allow_props(&entries, &["version", "name", "icon", "min-tier"])?;
        let id = self.single_string_arg(node, &entries, "definition id")?;
        if id.is_empty() || id.contains('@') || id.contains('/') {
            return Err(self.err_node(
                node,
                format!("definition id '{id}' must be non-empty, without '@' or '/'"),
            ));
        }
        let version_entry = entries.props.get("version").ok_or_else(|| {
            self.err_node(
                node,
                format!("definition '{id}' needs a version, e.g. version=\"1.0.0\""),
            )
        })?;
        let version = self.plain_string(version_entry, "version")?;
        if version.is_empty() || version.contains('@') {
            return Err(self.err_entry(
                version_entry,
                format!("version '{version}' must be non-empty, without '@'"),
            ));
        }
        let name = match entries.props.get("name") {
            Some(entry) => self.plain_string(entry, "name")?,
            None => id.clone(),
        };
        let mut meta = CharmMeta::new(name);
        if let Some(entry) = entries.props.get("icon") {
            meta.icon = Some(self.plain_string(entry, "icon")?);
        }
        if let Some(entry) = entries.props.get("min-tier") {
            let text = self.plain_string(entry, "min-tier")?;
            let tier: Tier = text
                .parse()
                .map_err(|e: crate::model::ParseTierError| self.err_entry(entry, e.to_string()))?;
            meta.min_tier = tier.0;
        }
        let mut network: Option<Network> = None;
        for child in children(node) {
            self.no_annotation(child)?;
            match child.name().value() {
                "capability" => {
                    if network.is_some() {
                        return Err(self.err_node(
                            child,
                            "list capabilities before the definition's `network`",
                        ));
                    }
                    if child.children().is_some() {
                        return Err(self.err_node(child, "`capability` takes no children block"));
                    }
                    let entries = self.entries(child)?;
                    self.allow_props(&entries, &[])?;
                    let capability = self.single_string_arg(child, &entries, "capability name")?;
                    if meta.capabilities.contains(&capability) {
                        return Err(self.err_node(
                            child,
                            format!("capability '{capability}' is listed twice"),
                        ));
                    }
                    meta.capabilities.push(capability);
                }
                "network" => {
                    if network.is_some() {
                        return Err(self.err_node(child, "a definition has only one `network`"));
                    }
                    let net = self.subnet_network(child)?;
                    if let Some((_, inner)) = net.walk().into_iter().find(|(_, n)| n.is_instance())
                    {
                        return Err(DocError::invalid(
                            inner.at.0,
                            format!(
                                "node '{}': Charm instances inside definitions are not supported yet",
                                inner.name
                            ),
                        ));
                    }
                    network = Some(net);
                }
                other => {
                    return Err(self.err_node(
                        child,
                        format!(
                            "unknown node `{other}` in def '{id}'; expected `capability` or `network`"
                        ),
                    ));
                }
            }
        }
        Ok(DocCharmDef {
            id,
            version,
            meta,
            network: network.unwrap_or_default(),
            at: SourceLoc(self.at_node(node)),
        })
    }

    /// The `def="id@version"` property of a node: required on
    /// `core.charm@1` nodes, refused elsewhere.
    pub(super) fn def_ref(
        &self,
        node: &KdlNode,
        entries: &Entries<'_>,
        doc_node: &mut DocNode,
    ) -> Result<(), DocError> {
        match entries.props.get("def") {
            None if doc_node.is_instance() => Err(self.err_node(
                node,
                format!(
                    "node '{}' is a Charm instance ({CHARM_TYPE}) and needs def=\"id@version\"",
                    doc_node.name
                ),
            )),
            None => Ok(()),
            Some(entry) if !doc_node.is_instance() => Err(self.err_entry(
                entry,
                format!("`def` is only allowed on Charm instances (type=\"{CHARM_TYPE}\")"),
            )),
            Some(entry) => {
                let text = self.plain_string(entry, "def")?;
                let Some((id, version)) = text
                    .rsplit_once('@')
                    .filter(|(i, v)| !i.is_empty() && !v.is_empty())
                else {
                    return Err(self.err_entry(
                        entry,
                        format!("def must be \"id@version\", found \"{text}\""),
                    ));
                };
                doc_node.def = Some(DocDefRef {
                    id: id.to_owned(),
                    version: version.to_owned(),
                    at: SourceLoc(self.at_entry(entry)),
                });
                Ok(())
            }
        }
    }

    /// Every instance must name a definition of the `charms` block at its
    /// version, and set only inputs its definition's interface has.
    pub(super) fn check_def_refs(
        &self,
        charms: &[DocCharmDef],
        network: &Network,
    ) -> Result<(), DocError> {
        let defs: BTreeMap<&str, &DocCharmDef> =
            charms.iter().map(|d| (d.id.as_str(), d)).collect();
        for (_, node) in network.walk() {
            let Some(r) = &node.def else {
                continue;
            };
            let def = defs.get(r.id.as_str()).ok_or_else(|| {
                DocError::invalid(
                    r.at.0,
                    format!(
                        "node '{}': unknown Charm definition '{}' (not in the `charms` block)",
                        node.name, r.id
                    ),
                )
            })?;
            if def.version != r.version {
                return Err(DocError::invalid(
                    r.at.0,
                    format!(
                        "node '{}': Charm definition '{}' is at version {}, not {}",
                        node.name, r.id, def.version, r.version
                    ),
                ));
            }
            let inputs: BTreeSet<&str> = def
                .network
                .nodes
                .iter()
                .filter(|n| n.type_id == livspeil_graph::INPUT_TYPE)
                .map(|n| n.name.as_str())
                .collect();
            if let Some(port) = node.ports().find(|p| !inputs.contains(p)) {
                return Err(DocError::invalid(
                    node.at.0,
                    format!(
                        "node '{}': definition '{}' has no interface input '{port}'",
                        node.name, r.id
                    ),
                ));
            }
        }
        Ok(())
    }
}
