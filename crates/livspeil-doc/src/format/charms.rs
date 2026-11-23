// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Writing shared Charm definitions (M0.16): the `charms` block. See the
//! crate documentation for the syntax.

use std::fmt::Write as _;

use super::{INDENT, write_network};
use crate::error::DocError;
use crate::model::{DocCharmDef, Tier};
use crate::value::quote;

/// The `charms` block (definitions sorted by id), or nothing.
pub(super) fn write_charms(out: &mut String, charms: &[DocCharmDef]) -> Result<(), DocError> {
    if charms.is_empty() {
        return Ok(());
    }
    let mut defs: Vec<&DocCharmDef> = charms.iter().collect();
    defs.sort_by(|a, b| a.id.cmp(&b.id));
    let _ = writeln!(out, "{INDENT}charms {{");
    for def in defs {
        let pad = INDENT.repeat(2);
        let _ = write!(
            out,
            "{pad}def {} version={}",
            quote(&def.id),
            quote(&def.version)
        );
        if def.meta.name != def.id {
            let _ = write!(out, " name={}", quote(&def.meta.name));
        }
        if let Some(icon) = &def.meta.icon {
            let _ = write!(out, " icon={}", quote(icon));
        }
        if def.meta.min_tier != 0 {
            let _ = write!(
                out,
                " min-tier={}",
                quote(&Tier(def.meta.min_tier).to_string())
            );
        }
        out.push_str(" {\n");
        for capability in &def.meta.capabilities {
            let _ = writeln!(out, "{pad}{INDENT}capability {}", quote(capability));
        }
        let mut network = def.network.clone();
        network.sort();
        let _ = write!(out, "{pad}{INDENT}network");
        write_network(out, &network, 3)?;
        let _ = writeln!(out, "{pad}}}");
    }
    let _ = writeln!(out, "{INDENT}}}");
    Ok(())
}
