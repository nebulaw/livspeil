// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! System bindings for the cooker (milestone M0.8).
//!
//! A [`Cooker`] can hold a [`SystemHost`] ([`Cooker::set_system`]). Nodes
//! that read the system ([`NodeImpl::reads_system`]) get it through
//! [`NodeImpl::cook_with_system`]; other nodes are unaffected.
//!
//! # Caching
//!
//! A system-reading node is a pure function of its inputs and the host
//! snapshot, identified by the *system key* `(host generation, host
//! revision)`: the generation changes with every [`Cooker::set_system`]
//! call, the revision is [`SystemHost::revision`], polled once per cook.
//! When the key differs from the one the cache was built with, the cooker
//! invalidates the system-reading nodes and everything downstream of them
//! (through links and `ch()` readers) before cooking. The system-reading
//! nodes are then re-cooked; their dependents follow the usual rules, so
//! a listing that did not change stops there (early cutoff). Nothing is
//! invalidated while the key stays the same, so a host that never bumps
//! its revision gets each listing cooked once per input change.
//!
//! The invalidation uses the cooker's own bookkeeping only (the graph is
//! not touched): a node is stale when `validated < dirty`, and every live
//! node has `dirty >= 1` (graph epochs start at 1), so `validated = 0`
//! makes it stale; `rev = u64::MAX` (a revision no node reaches) forces a
//! system-reading node to re-cook instead of being reused.

use std::sync::Arc;

use livspeil_node_api::{NodeImpl, SystemContext, SystemHost};

use super::Cooker;
use crate::graph::Graph;

/// The host and the system key the cache was validated with.
#[derive(Debug, Default)]
pub(super) struct SystemState {
    host: Option<Arc<dyn SystemHost>>,
    /// Bumped by every `set_system`.
    generation: u64,
    /// `(generation, revision)` the cached results belong to.
    synced: Option<(u64, u64)>,
}

impl SystemState {
    /// The context handed to nodes.
    pub(super) fn context(&self) -> SystemContext<'_> {
        match &self.host {
            Some(host) => SystemContext::new(host.as_ref()),
            None => SystemContext::none(),
        }
    }

    fn key(&self) -> (u64, u64) {
        (
            self.generation,
            self.host.as_ref().map_or(0, |h| h.revision()),
        )
    }
}

impl Cooker {
    /// Install (or with `None`, remove) the host whose system bindings
    /// system-reading nodes see. Every system-reading node is re-cooked at
    /// the next cook, as if the host's revision had changed.
    pub fn set_system(&mut self, host: Option<Arc<dyn SystemHost>>) {
        self.system.host = host;
        self.system.generation += 1;
    }

    /// The installed host, if any.
    #[must_use]
    pub fn system(&self) -> Option<&Arc<dyn SystemHost>> {
        self.system.host.as_ref()
    }

    /// Invalidate system-reading nodes and their downstream when the system
    /// key changed since the last cook (see the module documentation).
    pub(super) fn sync_system(&mut self, graph: &Graph) {
        let key = self.system.key();
        if self.system.synced == Some(key) {
            return;
        }
        self.system.synced = Some(key);
        let mut stack: Vec<usize> = (0..graph.slot_count())
            .filter(|&slot| {
                graph
                    .node_at(slot)
                    .is_some_and(|n| NodeImpl::reads_system(n.imp.as_ref()))
            })
            .collect();
        for &slot in &stack {
            if let Some(Some(entry)) = self.entries.get_mut(slot) {
                entry.rev = u64::MAX;
            }
        }
        let mut seen = vec![false; graph.slot_count()];
        while let Some(slot) = stack.pop() {
            if std::mem::replace(&mut seen[slot], true) {
                continue;
            }
            if let Some(Some(entry)) = self.entries.get_mut(slot) {
                entry.validated = 0;
            }
            stack.extend(
                graph
                    .dependents_at(slot)
                    .iter()
                    .filter(|&&next| !seen[next]),
            );
        }
    }
}
