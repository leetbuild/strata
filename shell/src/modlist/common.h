#pragma once

// Small helpers shared by the modlist area's dialogs.

#include "rust/cxx.h"

#include <QString>

#include <string>

namespace strata::modlist
{

inline QString qs(const rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

inline std::string utf8(const QString& s)
{
  return s.toStdString();
}

inline rust::String rs(const QString& s)
{
  return rust::String(s.toStdString());
}

}  // namespace strata::modlist
