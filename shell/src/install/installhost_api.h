#pragma once

// What crates/strata-ffi/src/bridge_install.rs calls: the interface of the installer
// dialogs, without Qt (the bridge's generated code includes this header). The window's
// implementation is InstallDialogs (installhost.h).

#include "rust/cxx.h"

#include <cstdint>

namespace strata
{

class InstallHost
{
public:
  virtual ~InstallHost() = default;
  // Shows dialog `kind` with the JSON `request`; returns the JSON answer. Any thread.
  virtual rust::String ask(rust::Str kind, rust::Str request) = 0;
  // Extraction progress; false when the user cancelled. Any thread.
  virtual bool progress(std::uint8_t percent) = 0;
};

class DownloadHost
{
public:
  virtual ~DownloadHost() = default;
  // The archive's name, bytes so far and in all (0 when unknown); false pauses. Called on the
  // worker thread.
  virtual bool progress(rust::Str file, std::uint64_t done, std::uint64_t total) = 0;
};

class NexusLoginHost
{
public:
  virtual ~NexusLoginHost() = default;
  // A log line of the login; `url` is the page to open in the browser ("" for none). Called on
  // the worker thread.
  virtual void state(rust::Str line, rust::Str url) = 0;
};

}  // namespace strata
