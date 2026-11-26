// MO2's mod information dialog (modinfodialog.ui): text files, ini files, images, optional
// plugins, conflicts (general and advanced, Root files as /Root/...), categories, Nexus
// info (read only), notes, file tree. What it shows comes from strata_ui::conflicts::modinfo;
// every change goes back through it (bridge_conflicts.rs).
#pragma once

#include "strata_bridge/bridge_conflicts.h"

#include <QDialog>
#include <QHash>
#include <QStringListModel>

#include <QPointer>
#include <QThread>

class QListWidget;

#include <atomic>
#include <functional>
#include <memory>
#include <optional>

namespace Ui
{
class ModInfoDialog;
}

class QListView;
class QLineEdit;
class QModelIndex;
class QPlainTextEdit;
class QSortFilterProxyModel;
class QStandardItemModel;
class QTreeView;

namespace strata::conflicts
{

class ModInfoDialog : public QDialog
{
  Q_OBJECT

public:
  // The tabs, in MO2's order (ModInfoTabIDs).
  enum Tab
  {
    TextFiles = 0,
    IniFiles,
    Images,
    Esps,
    Conflicts,
    Categories,
    Nexus,
    Notes,
    Filetree
  };

  // Reads the mod; throws ::an exception when it can't (the caller shows the message).
  ModInfoDialog(const QString& instanceRoot, const QString& profile, const QString& mod,
                QWidget* parent = nullptr);
  ~ModInfoDialog() override;

  void selectTab(int tab);

  // Opens the dialog for `mod`, modal; reports an error in a message box. Returns whether
  // something of the mod changed on disk (the caller refreshes its lists).
  // `goToMod`, when given, adds "Go to" entries for the other mods of a conflict: the dialog
  // closes and it is called with the mod's name (MainWindow::selectMod).
  // `neighbour`, when given, shows "Previous" / "Next" (and PageUp / PageDown): it returns
  // the mod before (-1) or after (+1) a mod in the list as the user sees it, "" for none
  // (ModListView::prevMod / nextMod). The dialog saves, then shows that mod on the same tab.
  static bool open(QWidget* parent, const QString& instanceRoot, const QString& profile,
                   const QString& mod, int tab = -1,
                   std::function<void(const QString&)> goToMod = {},
                   std::function<QString(const QString&, int)> neighbour = {});

  bool changed() const { return m_changed; }

protected:
  void done(int result) override;

private:
  // A text or ini tab: its file list, filter and editor.
  struct FilesTab
  {
    QStringListModel model;
    QSortFilterProxyModel* filter = nullptr;
    QListView* list               = nullptr;
    QPlainTextEdit* editor        = nullptr;
    QString current;  // relative path of the file in the editor
  };

  void setupFilesTab(FilesTab& tab, QListView* list, QLineEdit* filterEdit, QPlainTextEdit* editor,
                     const QStringList& files);
  bool saveIfModified(FilesTab& tab);  // false: the user cancelled
  void setupImages();
  // loads the Images tab's thumbnails on a worker, first rows first; a new call cancels the last
  void loadThumbnails(QListWidget* list, const QStringList& paths);
  void stopThumbnails();
  void setupPlugins();
  void reloadPlugins();
  void setupConflicts();
  void fillAdvanced();
  void conflictMenu(QTreeView* tree, const QPoint& pos, int pathColumn);
  void setupCategories();
  void setupNexus();
  void setupNotes();
  void setupFiletree();
  void setTabColors();
  void save();

  std::unique_ptr<Ui::ModInfoDialog> ui;
  std::optional<::rust::Box<ModInfoView>> m_view;
  QString m_dir;
  QString m_mod;
  FilesTab m_text;
  FilesTab m_ini;
  QStringListModel m_activePlugins;
  QStringListModel m_inactivePlugins;
  // Plugins made active in this dialog: new path -> where they came from (ESPItem's
  // inactivePath), so deactivating puts them back.
  QHash<QString, QString> m_cameFrom;
  QStandardItemModel* m_advanced = nullptr;
  bool m_changed = false;
  std::function<void(const QString&)> m_goToMod;
  QString m_goTo;  // chosen in a "Go to" menu: called after the dialog closed
  int m_step = 0;  // Previous (-1) or Next (+1) was clicked: open() shows that mod
  QPointer<QThread> m_thumbWorker;
  std::shared_ptr<std::atomic<bool>> m_thumbCancel;
};

}  // namespace strata::conflicts
