#pragma once

#include <QFileSystemWatcher>
#include <QObject>
#include <QString>
#include <QTimer>

// Finds, applies and hot-reloads the .qss stylesheet (docs/DESIGN.md, "Style").
//
// The sheet is applied to the whole application only (never per widget, which would override
// it). The watcher follows the sheet through edits in place, replace by
// rename (what renderers do) and symlinks (desktop themes link its rendered sheets in).
class Style : public QObject
{
  Q_OBJECT

public:
  explicit Style(QObject* parent = nullptr);

  // The instance folder whose stylesheets/ is searched first ("" for none).
  void setInstance(const QString& root);

  // `name` as MO2 stores it in [Settings] style ("Night Eyes.qss"; ".qss" may be left
  // out), or a Qt style name ("Fusion"). Returns false when the sheet is not found; the
  // window then keeps the look it had.
  bool apply(const QString& name);

  // The sheet in use, as found ("" for none).
  QString file() const { return m_file; }

signals:
  // The sheet was applied again after it changed on disk.
  void reloaded(const QString& file);

private:
  void load();
  void watch();
  void onChanged();

  QString m_instance;
  QString m_file;
  QByteArray m_content;  // what was applied last
  QFileSystemWatcher m_watcher;
  QTimer m_debounce;
};
