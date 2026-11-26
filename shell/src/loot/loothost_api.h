#pragma once

// What crates/strata-ffi/src/bridge_loot.rs calls: the LOOT dialog's side of a sort, without
// Qt (the bridge's generated code includes this header). The dialog's implementation is
// LootDialog (lootdialog.h).

#include "rust/cxx.h"

#include <cstdint>

namespace strata
{

class LootHost
{
public:
  virtual ~LootHost() = default;
  // A step (lootcli::Progress numbers). Any thread.
  virtual void progress(std::uint8_t step) const = 0;
  // A log line; level 0 debug, 1 info, 2 warning, 3 error. Any thread (libloot's workers too).
  virtual void log(std::uint8_t level, rust::Str line) const = 0;
  // The user asked to stop. Any thread.
  virtual bool cancelled() const = 0;
};

}  // namespace strata
