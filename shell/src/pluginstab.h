#pragma once

// The Plugins tab (MO2's PluginListView / PluginList) and the Saves tab: the core's plugin
// list, scanned on a worker; checkbox, drag and drop, context menu and Sort through the core.
// The saves are read on the same worker, since they need the plugin list for the missing
// plugins.

#include "strata_bridge/bridge.h"

#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QSortFilterProxyModel>
#include <QStandardItemModel>

#include <memory>

namespace Ui
{
class MainWindow;
}

class PluginsTab : public QObject
{
  Q_OBJECT

public:
  PluginsTab(Ui::MainWindow* ui, QObject* parent = nullptr);

  // The instance and profile to show; reads them on a worker.
  void setSource(const QString& root, const QString& profile);
  // Reads the list again (after the mods changed: other plugins are available).
  void reload();

  // Test hooks.
  void toggleForTest(const QString& name);
  int rowCount() const { return m_model.rowCount(); }
  // A read of the list is in flight (the tests wait for it).
  bool loading() const { return m_loading; }

signals:
  void error(const QString& message);
  // The list was (re)filled from the core.
  void filled();

private:
  void fill();
  void fillArchives();
  void fillSaves(const rust::Vec<strata::SaveRow>& saves);
  void onItemChanged(QStandardItem* item);
  void onDrop(int row);
  void showContextMenu(const QPoint& pos);
  void sort();
  QStringList selected() const;
  template <class Op>
  void operation(const char* what, Op op);

  Ui::MainWindow* ui;
  QStandardItemModel m_model;
  QSortFilterProxyModel m_proxy;
  std::shared_ptr<rust::Box<strata::Plugins>> m_plugins;
  QString m_root;
  QString m_profile;
  int m_generation = 0;
  bool m_updating  = false;
  bool m_loading   = false;
  bool m_again     = false;  // changed while loading: read again when it is back
  // MO2's m_didUpdateMasterList: the masterlist is downloaded by the first sort only
  bool m_didUpdateMasterList = false;
  // the last LOOT report's plugin parts, by lower-case name (PluginList::m_AdditionalInfo)
  QHash<QString, QJsonObject> m_loot;
};
