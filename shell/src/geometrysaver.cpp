#include "geometrysaver.h"

#include "strata_bridge/bridge.h"

#include <QDialog>

namespace
{

QString key(const QDialog* dialog)
{
  return dialog->objectName() + "_geometry";  // widgetNameWithTopLevel of a top level
}

}  // namespace

GeometrySaver::GeometrySaver(const QString& root, QDialog* dialog) : m_root(root), m_dialog(dialog)
{
  if (m_root.isEmpty() || !dialog || dialog->objectName().isEmpty()) {
    return;
  }
  try {
    const auto session = strata::open_session(m_root.toStdString(), "");
    const QByteArray k = key(dialog).toUtf8();
    const auto value   = session->load_setting("Geometry", rust::Str(k.data(), std::size_t(k.size())));
    if (value.found) {
      dialog->restoreGeometry(
          QByteArray(reinterpret_cast<const char*>(value.bytes.data()), qsizetype(value.bytes.size())));
    }
  } catch (const std::exception& e) {
    qWarning("cannot read the size of %s: %s", qUtf8Printable(dialog->objectName()), e.what());
  }
}

GeometrySaver::~GeometrySaver()
{
  if (m_root.isEmpty() || !m_dialog || m_dialog->objectName().isEmpty()) {
    return;
  }
  try {
    const auto session = strata::open_session(m_root.toStdString(), "");
    strata::SettingValue v;
    v.section          = "Geometry";
    const QByteArray k = key(m_dialog).toUtf8();
    v.key              = rust::String(k.data(), std::size_t(k.size()));
    for (char c : m_dialog->saveGeometry()) {
      v.bytes.push_back(std::uint8_t(c));
    }
    v.kind = 1;  // a QByteArray, written as @ByteArray(...)
    rust::Vec<strata::SettingValue> values;
    values.push_back(std::move(v));
    session->store_settings(std::move(values));
  } catch (const std::exception& e) {
    qWarning("cannot store the size of %s: %s", qUtf8Printable(m_dialog->objectName()), e.what());
  }
}
