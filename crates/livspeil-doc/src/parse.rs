// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The reader: KDL v2 text to [`Document`].

use std::collections::{BTreeMap, BTreeSet};

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use livspeil_graph::{CharmMeta, NodeId};
use livspeil_node_api::{PortType, Value};
use livspeil_params::{AnimChannel, AnimError, Extrapolation, Interpolation, Keyframe, Tangent};
use uuid::Uuid;

use crate::anim;
use crate::error::{DocError, Location};
use crate::migrate::migrate;
use crate::model::{
    CURRENT_FORMAT, DocAnim, DocExpr, DocLink, DocNode, Document, Meta, Network, ROOT_NETWORK,
    SourceLoc, Tier,
};
use crate::value::decode;

// Block values (`table`, `curve`) inside `input` (M0.8).
mod blocks;
// Shared Charm definitions (`charms`, `def=`; M0.16).
mod charms;

/// Parse the text of a `room.kdl` file.
///
/// Documents in an older format are migrated to [`CURRENT_FORMAT`] first.
/// The reader is strict: unknown nodes, properties or annotations are
/// errors, so that typos never silently drop data.
///
/// # Errors
/// [`DocError::Syntax`] for invalid KDL, [`DocError::Invalid`] for a
/// structurally invalid document, [`DocError::UnsupportedFormat`] and
/// [`DocError::Migration`] for format problems. Errors carry the line and
/// column when known.
pub fn parse(text: &str) -> Result<Document, DocError> {
    let lines = LineIndex::new(text);
    let kdl = KdlDocument::parse_v2(text).map_err(|err| {
        let first = err.diagnostics.first();
        DocError::Syntax {
            at: lines.locate(first.map_or(0, |d| d.span.offset())),
            message: first.map_or_else(
                || "invalid KDL".to_owned(),
                |d| {
                    let mut msg = d.to_string();
                    if let Some(label) = &d.label {
                        msg = format!("{msg} ({label})");
                    }
                    if let Some(help) = &d.help {
                        msg = format!("{msg}; {help}");
                    }
                    msg
                },
            ),
        }
    })?;
    Reader { lines }.document(kdl)
}

/// Maps byte offsets to 1-based line and column (counted in characters).
struct LineIndex<'a> {
    text: &'a str,
}

impl<'a> LineIndex<'a> {
    fn new(text: &'a str) -> Self {
        LineIndex { text }
    }

    fn locate(&self, offset: usize) -> Location {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let before = &self.text[..offset];
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        let column = before[line_start..].chars().count() + 1;
        Location { line, column }
    }
}

struct Reader<'a> {
    lines: LineIndex<'a>,
}

/// The entries of a KDL node split into positional arguments and
/// properties, with duplicate properties rejected.
struct Entries<'n> {
    args: Vec<&'n KdlEntry>,
    props: BTreeMap<&'n str, &'n KdlEntry>,
}

impl Reader<'_> {
    fn at_node(&self, node: &KdlNode) -> Option<Location> {
        Some(self.lines.locate(node.span().offset()))
    }

    fn at_entry(&self, entry: &KdlEntry) -> Option<Location> {
        Some(self.lines.locate(entry.span().offset()))
    }

    fn err_node(&self, node: &KdlNode, message: impl Into<String>) -> DocError {
        DocError::invalid(self.at_node(node), message)
    }

    fn err_entry(&self, entry: &KdlEntry, message: impl Into<String>) -> DocError {
        DocError::invalid(self.at_entry(entry), message)
    }

    fn document(&self, kdl: KdlDocument) -> Result<Document, DocError> {
        let mut nodes = kdl.nodes().iter();
        let Some(room) = nodes.next() else {
            return Err(DocError::invalid(
                None,
                "empty document; expected `room \"Name\" format=1 { ... }`",
            ));
        };
        if room.name().value() != "room" {
            return Err(self.err_node(
                room,
                format!(
                    "expected the `room` header node, found `{}`",
                    room.name().value()
                ),
            ));
        }
        if let Some(extra) = nodes.next() {
            return Err(self.err_node(
                extra,
                "only one top-level node (`room`) is allowed; put everything inside it",
            ));
        }
        self.no_annotation(room)?;

        let found = {
            let entries = self.entries(room)?;
            let entry = entries.props.get("format").ok_or_else(|| {
                self.err_node(room, "the room header needs a format, e.g. `format=1`")
            })?;
            match (entry.ty(), entry.value()) {
                (None, KdlValue::Integer(v)) => *v,
                _ => {
                    return Err(self.err_entry(entry, "format must be an integer, e.g. `format=1`"));
                }
            }
        };
        let from = u32::try_from(found)
            .ok()
            .filter(|v| (1..=CURRENT_FORMAT).contains(v))
            .ok_or(DocError::UnsupportedFormat {
                found,
                newest: CURRENT_FORMAT,
            })?;
        let mut room = room.clone();
        migrate(&mut room, from)?;
        self.room(&room)
    }

    fn room(&self, room: &KdlNode) -> Result<Document, DocError> {
        let entries = self.entries(room)?;
        self.allow_props(&entries, &["format", "min-tier"])?;
        let name = self.single_string_arg(room, &entries, "room name")?;
        let min_tier = match entries.props.get("min-tier") {
            None => Tier::default(),
            Some(entry) => {
                let text = self.plain_string(entry, "min-tier")?;
                text.parse().map_err(|e: crate::model::ParseTierError| {
                    self.err_entry(entry, e.to_string())
                })?
            }
        };

        let mut network = None;
        let mut meta = None;
        let mut charms = None;
        for child in children(room) {
            self.no_annotation(child)?;
            match child.name().value() {
                "network" => {
                    if network.is_some() {
                        return Err(self.err_node(
                            child,
                            "only one network is allowed in the room (subnets nest theirs inside their `node`)",
                        ));
                    }
                    network = Some(self.root_network(child)?);
                }
                "charms" => {
                    if charms.is_some() {
                        return Err(self.err_node(child, "only one charms block is allowed"));
                    }
                    charms = Some(self.charms(child)?);
                }
                "meta" => {
                    if meta.is_some() {
                        return Err(self.err_node(child, "only one meta block is allowed"));
                    }
                    meta = Some(self.meta(child)?);
                }
                other => {
                    return Err(self.err_node(
                        child,
                        format!(
                            "unknown node `{other}` in room; expected `charms`, `network` or `meta`"
                        ),
                    ));
                }
            }
        }
        let network = network.ok_or_else(|| {
            self.err_node(room, "the room has no network; add `network \"/\" { ... }`")
        })?;
        let meta = meta.unwrap_or_default();
        let charms = charms.unwrap_or_default();
        self.check_def_refs(&charms, &network)?;

        let paths = network.walk();
        let names: BTreeSet<&str> = paths.iter().map(|(p, _)| p.as_str()).collect();
        if let Some(unknown) = meta.positions.keys().find(|n| !names.contains(n.as_str())) {
            return Err(DocError::invalid(
                children(room)
                    .find(|c| c.name().value() == "meta")
                    .and_then(|c| self.at_node(c)),
                format!("meta refers to unknown node '{unknown}'"),
            ));
        }

        Ok(Document {
            name,
            format: CURRENT_FORMAT,
            min_tier,
            charms,
            network,
            meta,
        })
    }

    /// `network "/" { ... }`.
    fn root_network(&self, node: &KdlNode) -> Result<Network, DocError> {
        let entries = self.entries(node)?;
        self.allow_props(&entries, &[])?;
        let path = self.single_string_arg(node, &entries, "network path")?;
        if path != ROOT_NETWORK {
            return Err(self.err_node(
                node,
                format!(
                    "network '{path}': the room's network is \"/\"; subnets nest their `network` block inside their `node`"
                ),
            ));
        }
        self.network(node)
    }

    /// A subnet's `network { ... }`, without arguments.
    fn subnet_network(&self, node: &KdlNode) -> Result<Network, DocError> {
        let entries = self.entries(node)?;
        self.allow_props(&entries, &[])?;
        if let Some(arg) = entries.args.first() {
            return Err(self.err_entry(
                arg,
                "a subnet's `network` takes no path; it is the network of the enclosing node",
            ));
        }
        self.network(node)
    }

    fn network(&self, node: &KdlNode) -> Result<Network, DocError> {
        let mut network = Network::default();
        let mut names = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for child in children(node) {
            self.no_annotation(child)?;
            match child.name().value() {
                "node" => {
                    let n = self.node(child)?;
                    if !names.insert(n.name.clone()) {
                        return Err(
                            self.err_node(child, format!("duplicate node name '{}'", n.name))
                        );
                    }
                    if !ids.insert(n.id) {
                        return Err(self.err_node(child, format!("duplicate node id {}", n.id)));
                    }
                    network.nodes.push(n);
                }
                "link" => network.links.push(self.link(child)?),
                other => {
                    return Err(self.err_node(
                        child,
                        format!("unknown node `{other}` in network; expected `node` or `link`"),
                    ));
                }
            }
        }
        Ok(network)
    }

    fn node(&self, node: &KdlNode) -> Result<DocNode, DocError> {
        let entries = self.entries(node)?;
        self.allow_props(&entries, &["type", "id", "port-type", "def"])?;
        let name = self.single_string_arg(node, &entries, "node name")?;
        if name.is_empty() {
            return Err(self.err_node(node, "node names must not be empty"));
        }
        let type_entry = entries.props.get("type").ok_or_else(|| {
            self.err_node(
                node,
                format!("node '{name}' needs a type, e.g. type=\"core.math.add@1\""),
            )
        })?;
        let type_id = self.plain_string(type_entry, "type")?;
        let id_entry = entries
            .props
            .get("id")
            .ok_or_else(|| self.err_node(node, format!("node '{name}' needs an id (a UUID)")))?;
        let id_text = self.plain_string(id_entry, "id")?;
        let id = Uuid::parse_str(&id_text)
            .map(NodeId::from_uuid)
            .map_err(|e| self.err_entry(id_entry, format!("invalid node id '{id_text}': {e}")))?;

        let mut doc_node = DocNode::new(id, name, type_id);
        doc_node.at = SourceLoc(self.at_node(node));
        if let Some(entry) = entries.props.get("port-type") {
            let text = self.plain_string(entry, "port-type")?;
            let ty = PortType::ALL
                .into_iter()
                .find(|t| t.name() == text)
                .ok_or_else(|| {
                    self.err_entry(
                        entry,
                        format!("unknown port type \"{text}\" (for example \"Float\", \"Color\")"),
                    )
                })?;
            doc_node.port_type = Some(ty);
        }
        self.def_ref(node, &entries, &mut doc_node)?;
        let mut seen = BTreeSet::new();
        for child in children(node) {
            self.no_annotation(child)?;
            match child.name().value() {
                "input" => self.input(child, &mut doc_node, &mut seen)?,
                "charm" | "network" if !doc_node.is_subnet() => {
                    return Err(self.err_node(
                        child,
                        format!(
                            "`{}` is only allowed in a subnet (type=\"{}\"); node '{}' is a {}",
                            child.name().value(),
                            livspeil_graph::SUBNET_TYPE,
                            doc_node.name,
                            doc_node.type_id
                        ),
                    ));
                }
                "charm" => {
                    if doc_node.charm.is_some() {
                        return Err(self.err_node(child, "only one `charm` block is allowed"));
                    }
                    doc_node.charm = Some(self.charm(child)?);
                }
                "network" => {
                    if doc_node.network.is_some() {
                        return Err(self.err_node(child, "a subnet has only one `network`"));
                    }
                    doc_node.network = Some(self.subnet_network(child)?);
                }
                other => {
                    return Err(self.err_node(
                        child,
                        format!(
                            "unknown node `{other}` in node '{}'; expected `input`, `charm` or `network`",
                            doc_node.name
                        ),
                    ));
                }
            }
        }
        Ok(doc_node)
    }

    /// `charm "Name" [version=".."] [icon=".."] [min-tier="T<n>"] {
    /// capability ".." }`.
    fn charm(&self, node: &KdlNode) -> Result<CharmMeta, DocError> {
        let entries = self.entries(node)?;
        self.allow_props(&entries, &["version", "icon", "min-tier"])?;
        let name = self.single_string_arg(node, &entries, "Charm name")?;
        let mut charm = CharmMeta::new(name);
        if let Some(entry) = entries.props.get("version") {
            charm.version = Some(self.plain_string(entry, "version")?);
        }
        if let Some(entry) = entries.props.get("icon") {
            charm.icon = Some(self.plain_string(entry, "icon")?);
        }
        if let Some(entry) = entries.props.get("min-tier") {
            let text = self.plain_string(entry, "min-tier")?;
            let tier: Tier = text
                .parse()
                .map_err(|e: crate::model::ParseTierError| self.err_entry(entry, e.to_string()))?;
            charm.min_tier = tier.0;
        }
        for child in children(node) {
            self.no_annotation(child)?;
            if child.name().value() != "capability" {
                return Err(self.err_node(
                    child,
                    format!(
                        "unknown node `{}` in charm; expected `capability \"name\"`",
                        child.name().value()
                    ),
                ));
            }
            if child.children().is_some() {
                return Err(self.err_node(child, "`capability` takes no children block"));
            }
            let entries = self.entries(child)?;
            self.allow_props(&entries, &[])?;
            let capability = self.single_string_arg(child, &entries, "capability name")?;
            if charm.capabilities.contains(&capability) {
                return Err(
                    self.err_node(child, format!("capability '{capability}' is listed twice"))
                );
            }
            charm.capabilities.push(capability);
        }
        Ok(charm)
    }

    /// `input "port" [<value>] [expr="..."] [{ anim ... }]`.
    fn input(
        &self,
        child: &KdlNode,
        node: &mut DocNode,
        seen: &mut BTreeSet<String>,
    ) -> Result<(), DocError> {
        const USAGE: &str = "expected `input \"port\" <value>` (or a `table`/`curve` block), optionally with `expr=\"...\"` and an `anim { ... }` block";
        let entries = self.entries(child)?;
        self.allow_props(&entries, &["expr"])?;
        let (port, value) = match entries.args.as_slice() {
            [port] => (port, None),
            [port, value] => (port, Some(value)),
            _ => return Err(self.err_node(child, USAGE)),
        };
        let port = self.plain_string(port, "input port name")?;
        if !seen.insert(port.clone()) {
            return Err(self.err_node(child, format!("input '{port}' is set more than once")));
        }
        if let Some(value) = value {
            let decoded = decode(value.ty().map(|t| t.value()), value.value())
                .map_err(|msg| self.err_entry(value, format!("input '{port}': {msg}")))?;
            node.inputs.insert(port.clone(), decoded);
        }
        if let Some(entry) = entries.props.get("expr") {
            let source = self.plain_string(entry, "expr")?;
            node.expressions.insert(
                port.clone(),
                DocExpr {
                    source,
                    at: SourceLoc(self.at_entry(entry)),
                },
            );
        }
        let mut anim = None;
        let mut block = false;
        for sub in children(child) {
            self.no_annotation(sub)?;
            if blocks::BLOCKS.contains(&sub.name().value()) {
                if value.is_some() || block {
                    return Err(self.err_node(
                        sub,
                        format!("input '{port}' has more than one static value"),
                    ));
                }
                block = true;
                let decoded = self.block_value(sub, &port)?;
                node.inputs.insert(port.clone(), decoded);
                continue;
            }
            if sub.name().value() != "anim" {
                return Err(self.err_node(
                    sub,
                    format!(
                        "unknown node `{}` in input '{port}'; expected `table`, `curve` or `anim`",
                        sub.name().value()
                    ),
                ));
            }
            if anim.is_some() {
                return Err(self.err_node(
                    sub,
                    format!("input '{port}' has more than one `anim` block"),
                ));
            }
            anim = Some(self.anim(sub, &port)?);
        }
        let has_anim = anim.is_some();
        if let Some(anim) = anim {
            node.animations.insert(port.clone(), anim);
        }
        if value.is_none() && !block && !entries.props.contains_key("expr") && !has_anim {
            return Err(self.err_node(child, format!("input '{port}' sets nothing; {USAGE}")));
        }
        Ok(())
    }

    /// `anim [pre=".."] [post=".."] [offset=<number>] { key ... }`.
    fn anim(&self, node: &KdlNode, port: &str) -> Result<DocAnim, DocError> {
        let entries = self.entries(node)?;
        self.allow_props(&entries, &["pre", "post", "offset"])?;
        if let Some(arg) = entries.args.first() {
            return Err(self.err_entry(
                arg,
                "`anim` takes no arguments; settings are `pre=`, `post=` and `offset=`",
            ));
        }
        let extrapolation = |key: &str| -> Result<Extrapolation, DocError> {
            let Some(entry) = entries.props.get(key) else {
                return Ok(Extrapolation::Hold);
            };
            let name = self.plain_string(entry, key)?;
            anim::extrapolation(&name).ok_or_else(|| {
                self.err_entry(
                    entry,
                    format!(
                        "unknown {key} extrapolation \"{name}\"; expected one of {}",
                        anim::list(anim::EXTRAPOLATIONS.map(|(n, _)| n))
                    ),
                )
            })
        };
        let pre = extrapolation("pre")?;
        let post = extrapolation("post")?;
        let offset = match entries.props.get("offset") {
            None => 0.0,
            Some(entry) => self.number(entry, "offset")?,
        };

        let mut keys = Vec::new();
        let mut key_nodes = Vec::new();
        for child in children(node) {
            self.no_annotation(child)?;
            if child.name().value() != "key" {
                return Err(self.err_node(
                    child,
                    format!(
                        "unknown node `{}` in anim; expected `key <time> <value>`",
                        child.name().value()
                    ),
                ));
            }
            keys.push(self.key(child)?);
            key_nodes.push(child);
        }
        let channel = AnimChannel::new(keys)
            .and_then(|c| c.with_extrapolation(pre, post).with_offset(offset))
            .map_err(|e| {
                let at = match &e {
                    AnimError::UnsupportedType { index, .. }
                    | AnimError::MixedTypes { index, .. }
                    | AnimError::NotIncreasing { index }
                    | AnimError::NonFinite { index }
                    | AnimError::InvalidSpring { index } => key_nodes.get(*index).copied(),
                    _ => None,
                };
                DocError::invalid(
                    self.at_node(at.unwrap_or(node)),
                    format!("input '{port}' anim: {e}"),
                )
            })?;
        Ok(DocAnim {
            channel,
            at: SourceLoc(self.at_node(node)),
        })
    }

    /// `key <time> <value> [interp=".."] [frequency=.. damping=..]
    /// [tangent=".."] [in=<value> out=<value>]`.
    fn key(&self, node: &KdlNode) -> Result<Keyframe, DocError> {
        if node.children().is_some() {
            return Err(self.err_node(node, "`key` takes no children block"));
        }
        let entries = self.entries(node)?;
        self.allow_props(
            &entries,
            &["interp", "frequency", "damping", "tangent", "in", "out"],
        )?;
        let [time, value] = entries.args.as_slice() else {
            return Err(self.err_node(node, "expected `key <time> <value>`"));
        };
        let time = self.number(time, "key time")?;
        let value = self.key_value(value, "key value")?;

        let name = |key: &str| -> Result<Option<(String, &KdlEntry)>, DocError> {
            entries
                .props
                .get(key)
                .map(|entry| self.plain_string(entry, key).map(|s| (s, *entry)))
                .transpose()
        };
        let only_with = |props: &[&str], setting: &str| -> Result<(), DocError> {
            match props.iter().find_map(|p| entries.props.get(p)) {
                Some(entry) => Err(self.err_entry(
                    entry,
                    format!(
                        "`{}` is only allowed with {setting}",
                        entry.name().map_or("", |n| n.value())
                    ),
                )),
                None => Ok(()),
            }
        };
        let required = |prop: &str, setting: &str| -> Result<&KdlEntry, DocError> {
            entries
                .props
                .get(prop)
                .copied()
                .ok_or_else(|| self.err_node(node, format!("{setting} needs `{prop}=`")))
        };

        let interpolation = match name("interp")? {
            None => Interpolation::Linear,
            Some((n, entry)) => match n.as_str() {
                "step" => Interpolation::Step,
                "linear" => Interpolation::Linear,
                "bezier" => Interpolation::Bezier,
                "spring" => Interpolation::Spring {
                    frequency: self
                        .number(required("frequency", "interp=\"spring\"")?, "frequency")?,
                    damping: self.number(required("damping", "interp=\"spring\"")?, "damping")?,
                },
                _ => {
                    return Err(self.err_entry(
                        entry,
                        format!(
                            "unknown interpolation \"{n}\"; expected one of {}",
                            anim::list(anim::INTERPOLATIONS)
                        ),
                    ));
                }
            },
        };
        if !matches!(interpolation, Interpolation::Spring { .. }) {
            only_with(&["frequency", "damping"], "interp=\"spring\"")?;
        }

        let tangent = match name("tangent")? {
            None => Tangent::Auto,
            Some((n, entry)) => match n.as_str() {
                "auto" => Tangent::Auto,
                "flat" => Tangent::Flat,
                "explicit" => Tangent::Explicit {
                    incoming: self
                        .key_value(required("in", "tangent=\"explicit\"")?, "in tangent")?,
                    outgoing: self
                        .key_value(required("out", "tangent=\"explicit\"")?, "out tangent")?,
                },
                _ => {
                    return Err(self.err_entry(
                        entry,
                        format!(
                            "unknown tangent \"{n}\"; expected one of {}",
                            anim::list(anim::TANGENTS)
                        ),
                    ));
                }
            },
        };
        if !matches!(tangent, Tangent::Explicit { .. }) {
            only_with(&["in", "out"], "tangent=\"explicit\"")?;
        }

        Ok(Keyframe {
            time,
            value,
            interpolation,
            tangent,
        })
    }

    /// A finite number; integers are accepted and stored as floats.
    fn number(&self, entry: &KdlEntry, what: &str) -> Result<f64, DocError> {
        match self.key_value(entry, what)? {
            Value::Float(v) => Ok(v),
            _ => Err(self.err_entry(entry, format!("{what} must be a number"))),
        }
    }

    /// A value inside an animation: the usual encoding, with integers read
    /// as floats (integers cannot be animated, so this is unambiguous).
    fn key_value(&self, entry: &KdlEntry, what: &str) -> Result<Value, DocError> {
        match decode(entry.ty().map(|t| t.value()), entry.value()) {
            Ok(Value::Int(v)) => Ok(Value::Float(v as f64)),
            Ok(v) => Ok(v),
            Err(msg) => Err(self.err_entry(entry, format!("{what}: {msg}"))),
        }
    }

    fn link(&self, node: &KdlNode) -> Result<DocLink, DocError> {
        if node.children().is_some() {
            return Err(self.err_node(node, "`link` takes no children block"));
        }
        let entries = self.entries(node)?;
        self.allow_props(&entries, &["from", "to"])?;
        if let Some(arg) = entries.args.first() {
            return Err(self.err_entry(
                arg,
                "links are written `link from=\"node.port\" to=\"node.port\"`",
            ));
        }
        let endpoint = |key: &str| -> Result<(String, String), DocError> {
            let entry = entries
                .props
                .get(key)
                .ok_or_else(|| self.err_node(node, format!("link needs `{key}=\"node.port\"`")))?;
            let text = self.plain_string(entry, key)?;
            match text.rsplit_once('.') {
                Some((n, p)) if !n.is_empty() && !p.is_empty() => Ok((n.to_owned(), p.to_owned())),
                _ => Err(self.err_entry(
                    entry,
                    format!("`{key}` must be \"node.port\", found \"{text}\""),
                )),
            }
        };
        let (from_node, from_port) = endpoint("from")?;
        let (to_node, to_port) = endpoint("to")?;
        Ok(DocLink {
            from_node,
            from_port,
            to_node,
            to_port,
            at: SourceLoc(self.at_node(node)),
        })
    }

    fn meta(&self, node: &KdlNode) -> Result<Meta, DocError> {
        let entries = self.entries(node)?;
        self.allow_props(&entries, &[])?;
        if let Some(arg) = entries.args.first() {
            return Err(self.err_entry(arg, "`meta` takes no arguments"));
        }
        let mut meta = Meta::default();
        for child in children(node) {
            self.no_annotation(child)?;
            if child.name().value() != "node" {
                return Err(self.err_node(
                    child,
                    format!(
                        "unknown node `{}` in meta; expected `node \"name\" x=.. y=..`",
                        child.name().value()
                    ),
                ));
            }
            let entries = self.entries(child)?;
            self.allow_props(&entries, &["x", "y"])?;
            let name = self.single_string_arg(child, &entries, "node name")?;
            let coord = |key: &str| -> Result<f64, DocError> {
                let entry = entries.props.get(key).ok_or_else(|| {
                    self.err_node(child, format!("meta node '{name}' needs `{key}=`"))
                })?;
                match (entry.ty(), entry.value()) {
                    // Integers are accepted and stored as floats.
                    (None, KdlValue::Integer(v)) => Ok(*v as f64),
                    (None, KdlValue::Float(v)) if v.is_finite() => Ok(*v),
                    _ => Err(self.err_entry(entry, format!("`{key}` must be a finite number"))),
                }
            };
            let position = [coord("x")?, coord("y")?];
            if meta.positions.insert(name.clone(), position).is_some() {
                return Err(
                    self.err_node(child, format!("meta lists node '{name}' more than once"))
                );
            }
        }
        Ok(meta)
    }

    fn entries<'n>(&self, node: &'n KdlNode) -> Result<Entries<'n>, DocError> {
        let mut entries = Entries {
            args: Vec::new(),
            props: BTreeMap::new(),
        };
        for entry in node.entries() {
            match entry.name() {
                None => entries.args.push(entry),
                Some(key) => {
                    if entries.props.insert(key.value(), entry).is_some() {
                        return Err(self.err_entry(
                            entry,
                            format!("property `{}` is given more than once", key.value()),
                        ));
                    }
                }
            }
        }
        Ok(entries)
    }

    fn allow_props(&self, entries: &Entries<'_>, allowed: &[&str]) -> Result<(), DocError> {
        match entries.props.iter().find(|(key, _)| !allowed.contains(key)) {
            None => Ok(()),
            Some((key, entry)) => Err(self.err_entry(
                entry,
                if allowed.is_empty() {
                    format!("unknown property `{key}`; this node takes no properties")
                } else {
                    format!(
                        "unknown property `{key}`; expected one of: {}",
                        allowed.join(", ")
                    )
                },
            )),
        }
    }

    fn single_string_arg(
        &self,
        node: &KdlNode,
        entries: &Entries<'_>,
        what: &str,
    ) -> Result<String, DocError> {
        match entries.args.as_slice() {
            [arg] => self.plain_string(arg, what),
            [] => Err(self.err_node(
                node,
                format!("`{}` needs a {what} string", node.name().value()),
            )),
            [_, extra, ..] => Err(self.err_entry(
                extra,
                format!("`{}` takes a single {what}", node.name().value()),
            )),
        }
    }

    fn plain_string(&self, entry: &KdlEntry, what: &str) -> Result<String, DocError> {
        match (entry.ty(), entry.value()) {
            (None, KdlValue::String(s)) => Ok(s.clone()),
            (Some(_), _) => Err(self.err_entry(entry, format!("{what} takes no type annotation"))),
            _ => Err(self.err_entry(entry, format!("{what} must be a string"))),
        }
    }

    fn no_annotation(&self, node: &KdlNode) -> Result<(), DocError> {
        match node.ty() {
            None => Ok(()),
            Some(ty) => Err(self.err_node(
                node,
                format!(
                    "unexpected type annotation ({}) on `{}`",
                    ty.value(),
                    node.name().value()
                ),
            )),
        }
    }
}

fn children(node: &KdlNode) -> impl Iterator<Item = &KdlNode> {
    node.children().into_iter().flat_map(KdlDocument::nodes)
}
