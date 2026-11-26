#pragma once

// The Data tab: the merged Data folder with each file's origin (MO2's DataTab /
// FileTreeModel). The tree is scanned on a worker thread and folders are filled in as the
// user opens them.

#include "strata_bridge/bridge.h"

#include <QObject>
#include <QStandardItemModel>

#include <memory>
#include <optional>

class QCheckBox;
class QLineEdit;
class QTreeView;
class QPushButton;

class DataTab : public QObject
{
  Q_OBJECT

public:
  // `area` 0: the Data tab; 1: the Root tab (Leet change 5). `filter`, `conflictsOnly` and
  // `archives` are the tab's filter row (may be null).
  DataTab(QTreeView* tree, QPushButton* refresh, int area, QLineEdit* filter,
          QCheckBox* conflictsOnly, QCheckBox* archives = nullptr, QObject* parent = nullptr);
  ~DataTab() override;

  // Which instance and profile to show; the scan starts the first time the tab is shown
  // (activate()) or right away when `now`.
  void setSource(const QString& root, const QString& profile);
  // The tab became visible (MainWindow::on_tabWidget_currentChanged → DataTab::activated).
  void activate();
  void refresh();

  // The header, so its state can be saved and restored.
  QStandardItemModel& model() { return m_model; }

private:
  void fill(QStandardItem* folder, const QString& path);
  void onExpanded(const QModelIndex& index);
  // A file's path below Data, "" for folders (conflicts area: the preview).
  QString filePath(const QModelIndex& index) const;

  void refill();

  QTreeView* m_tree;
  int m_area;
  QString m_filter;
  bool m_conflictsOnly = false;
  bool m_archives      = false;  // the "Archives" box (the Data tab only)
  bool m_archivesRead  = false;  // the shown view has the archives' file lists
  QStandardItemModel m_model;
  QString m_root;
  QString m_profile;
  bool m_stale = true;
  int m_generation = 0;  // drops the result of a scan that a newer one replaced
  std::shared_ptr<rust::Box<strata::DataView>> m_view;
};
