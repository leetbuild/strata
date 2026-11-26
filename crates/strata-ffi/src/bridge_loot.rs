//! The LOOT dialog's `cxx` bridge (`shell/src/loot/`): the Sort button runs on a worker
//! thread the shell starts and reports to the C++ `LootHost` (steps, log lines, cancel), then
//! returns MO2's report as JSON. Every function catches panics.

use std::sync::Arc;

use crate::guarded;

#[cxx::bridge(namespace = "strata")]
mod ffi {
    unsafe extern "C++" {
        include!("loot/loothost_api.h");

        /// The LOOT dialog's side of a sort. Its functions may be called from any thread.
        type LootHost;
        /// A step (`strata_loot::Progress`, lootcli's numbers).
        fn progress(self: &LootHost, step: u8);
        /// A log line; `level` 0 debug, 1 info, 2 warning, 3 error.
        fn log(self: &LootHost, level: u8, line: &str);
        /// The user pressed Cancel.
        fn cancelled(self: &LootHost) -> bool;
    }

    extern "Rust" {
        /// Sorts the profile's plugins with LOOT (on a worker) and writes the new order,
        /// downloading the masterlist first when `update_masterlist`;
        /// returns the report as JSON (`strata_ui::plugins::SortOutcome`: lootcli's
        /// `messages`, `plugins`, `stats`, and `moved`, `masterlistNote`). A cancelled sort is
        /// an error ("cancelled") and writes nothing.
        fn sort_plugins_report(root: &str, profile: &str, update_masterlist: bool, host: &LootHost) -> Result<String>;
    }
}

use ffi::LootHost;

/// The host, shared with libloot's log callback (which may run on its worker threads).
struct Host(*const LootHost);

// SAFETY: the C++ host's functions are thread safe (loothost_api.h), and the pointer is only
// used while `sort_plugins_report` runs: strata_loot drops its log callback (the only copy
// that outlives a step) before the sort returns.
unsafe impl Send for Host {}
unsafe impl Sync for Host {}

impl Host {
    fn get(&self) -> &LootHost {
        // SAFETY: see above; the reference lives as long as the call.
        unsafe { &*self.0 }
    }
}

impl strata_loot::Observer for Host {
    fn progress(&self, step: strata_loot::Progress) {
        self.get().progress(step as u8);
    }
    fn log(&self, level: strata_loot::Level, line: &str) {
        self.get().log(level as u8, line);
    }
    fn cancelled(&self) -> bool {
        self.get().cancelled()
    }
}

fn sort_plugins_report(root: &str, profile: &str, update_masterlist: bool, host: &LootHost) -> Result<String, String> {
    guarded(|| {
        let profile = (!profile.is_empty()).then_some(profile);
        let session = strata_ui::Session::open(std::path::Path::new(root), profile).map_err(|e| e.to_string())?;
        let observer = Arc::new(Host(host as *const LootHost));
        let outcome = session.sort_plugins_report(update_masterlist, observer.clone()).map_err(|e| e.to_string());
        if Arc::strong_count(&observer) != 1 {
            // libloot kept the callback: never let it reach a host that is gone
            return Err("internal error: the LOOT log callback outlived the sort".to_owned());
        }
        serde_json::to_string(&outcome?).map_err(|e| e.to_string())
    })
}
