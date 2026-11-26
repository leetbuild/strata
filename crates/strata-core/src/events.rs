//! Change events (`docs/ARCHITECTURE.md`, "Events"): what an operation changed, small
//! enough for a list model to update the rows concerned instead of resetting.
//!
//! Operations return their events; the caller (the CLI, or the ffi that turns them into Qt
//! signals) passes them on. They follow MO2's signals: `Profile::modStatusChanged`,
//! `ModList::modMoved`, `modRenamed`, `modPrioritiesChanged`, `ProfilesDialog::profile*`.

/// A change to a profile's mod list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModListEvent {
    /// These mods were switched on or off (`Profile::modStatusChanged`).
    StateChanged { names: Vec<String>, enabled: bool },
    /// One mod moved from one priority to another; the mods in between moved by one
    /// (`ModList::modMoved`). Several moves come in the order they happened.
    Moved { name: String, from: usize, to: usize },
    /// A separator's level changed (Sub-Separator): the tree under it changes.
    NestChanged { name: String, nest: u32 },
    /// A mod was renamed (`ModList::modRenamed`); its priority and state stay.
    Renamed { from: String, to: String },
    /// A mod was removed (its folder went to the trash or was deleted); the mods above it
    /// moved down by one.
    Removed { name: String, priority: usize },
    /// A mod was added at this priority (created, or a folder found on disk).
    Added { name: String, priority: usize },
    /// A mod's `meta.ini` changed (notes, colour, categories...): redraw its row.
    MetaChanged { name: String },
    /// Overwrite's content changed (cleared, or moved into a mod).
    OverwriteChanged,
}

/// A change to the instance's profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileEvent {
    Created { name: String },
    Renamed { from: String, to: String },
    Removed { name: String },
    /// The instance's selected profile changed.
    Selected { name: String },
    /// `settings.ini` changed (local saves, local inis).
    SettingsChanged { name: String },
    /// A backup of a profile file was made or restored.
    BackupChanged { name: String },
}
