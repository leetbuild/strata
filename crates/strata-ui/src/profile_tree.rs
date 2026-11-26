//! The merged tree of a profile and the plugins in it: the one way the CLI and the window
//! build them for reading.
//!
//! It lives here because it needs both the core (`resolve_layers`) and the overlay's
//! `Tree`, and neither of those may depend on the other (the core touches no FUSE crate, the
//! overlay knows no profiles); `strata-ui` is the lowest crate the CLI and the window share.

use strata_core::plugins::AvailablePlugin;
use strata_core::{Instance, Profile, resolve_layers};
use strata_game::plugin::is_plugin_name;
use strata_vfs::{Options, Tree};

use crate::{Error, Result};

/// The merged tree of `profile` as a deploy would show it (whiteouts applied), built for
/// reading: nothing is written. On a 1,200-mod list about 0.2 s: keep it off the UI thread.
pub fn merged_tree(instance: &Instance, profile: &Profile) -> Result<Tree> {
    let layers = resolve_layers(instance, profile)?;
    let whiteouts = instance.root.join("strata/whiteouts.txt");
    Tree::build(layers, Options::default(), Some(whiteouts)).map_err(Error::Tree)
}

/// The plugins at the top of the merged Data folder, with the layer each comes from.
pub fn available_plugins(instance: &Instance, tree: &Tree) -> Vec<AvailablePlugin> {
    let Some(data) = tree.resolve(instance.game.data_dir()) else { return Vec::new() };
    let mut out = Vec::new();
    for (id, node) in tree.children(data) {
        if node.is_dir() || !is_plugin_name(&node.name) {
            continue;
        }
        let (Some(path), Some(layer)) = (tree.real_path(id), tree.layer_of(id)) else { continue };
        out.push(AvailablePlugin { name: node.name.to_string(), origin: layer.id.clone(), path });
    }
    out
}
