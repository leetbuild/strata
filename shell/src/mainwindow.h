#pragma once

#include "downloadproxy.h"
#include "install/downloadactions.h"
#include "modlist.h"
#include "strata_bridge/bridge.h"

#include <QMainWindow>
#include <QSortFilterProxyModel>
#include <QHash>
#include <QSet>
#include <QStandardItemModel>

#include <functional>
#include <memory>
#include <optional>

namespace Ui
{
class MainWindow;
}

class ColumnFit;
class DataTab;
class FilterPanel;
class PluginsTab;
class LayoutState;
class QLabel;
class QMenu;
class QProcess;
class QUrl;
class Style;

// MO2's main window, built from MO2's own form (forms/mainwindow.ui). It owns no state of
// its own: every list is a view of what the core returns for the open session.
class MainWindow : public QMainWindow
{
  Q_OBJECT

public:
  explicit MainWindow(Style& style, QWidget* parent = nullptr);
  ~MainWindow() override;

  // Opens an instance and profile ("" = the selected one). On failure the window stays
  // empty and the error is in the log and the status bar.
  bool openInstance(const QString& root, const QString& profile);

  // File > Change Game: MO2's instance manager. A chosen instance is opened here when none
  // is open, else Strata restarts on it (MO2's ExitModOrganizer(Exit::Restart)).
  void showInstanceManager();
  // The list options menu's "Check for updates" (every Nexus mod of the instance).
  void checkForUpdates();
  void switchInstance(const QString& root);

  // Test hooks (STRATA_SHELL_TAB, STRATA_SHELL_HIDE_COLUMN): what a user does with a click.
  void selectTab(int index);
  void hideModColumn(int column);
  // Test hook: the profile combo set to `name` as a user would.
  void switchProfileForTest(const QString& name);
  // Test hooks: a click on a mod's checkbox; a drag of a mod onto a separator.
  void toggleModForTest(const QString& name);
  // Selects a mod in the list, opening its separators and scrolling to it ("go to" from the
  // mod info dialog's conflicts tabs).
  void selectMod(const QString& name);
  void tickFilterForTest(const QString& name);
  void togglePluginForTest(const QString& name);
  void showModColumn(int column);
  void dropOntoForTest(const QString& name, const QString& separator);

  // File > Refresh: everything read again from disk.
  void refresh();
  void applyInstanceStyle();

  // Runs one of the instance's executables through `strata run` in a child process; its
  // output goes to the log. `extra`: more options for `strata run`.
  void runExecutable(const QString& title, const QStringList& extra = {});


  // Opens one of the modlist area's dialogs ("profiles", "executables", "settings"; test
  // hook STRATA_SHELL_DIALOG) and reads the instance again when it changed something.
  void openDialog(const QString& name);
  // conflicts area: the mod information dialog of `mod` (tab -1: the first); reloads the
  // lists when something changed.
  void openModInfo(const QString& mod, int tab = -1);
  // An nxm:// link for the download queue (install area): from the browser through
  // `strata link` (listenForLinks) or a test.
  void addDownloadLink(const QString& url);
  // MO2's OverwriteInfoDialog (a double click on Overwrite).
  void openOverwrite();
  // conflicts area: the preview of a file of the merged Data folder (path below Data).
  void previewDataFile(const QString& path);
  // The mod before (step -1) or after (+1) `name` as the list shows it (the mod info
  // dialog's Previous / Next).
  QString neighbourMod(const QString& name, int step) const;

signals:
  // An archive dropped on the mod list (or installed at a row) is installed and moved where
  // it was dropped ("" when the install failed or was cancelled). The list is being read
  // again at this point.
  void dropInstallFinished(const QString& mod);

protected:
  void closeEvent(QCloseEvent* event) override;

private:
  void setupLists();
  void setupModList();
  void setupModListActions();
  void setupPluginsAndDownloads();
  // nxm:// links handed over by `strata link` (strata_ui::handover): a socket per window
  void listenForLinks();
  void setupDataTabs();
  void setupProfileRow();
  void setupRun();
  void setupMenus();
  void reloadAll();
  void loadModList();  // starts reading on a worker; showModList() when it is back
  void showModList();
  void loadExecutables();
  void loadDownloads();
  void loadProfiles();
  void updateModCounter();
  void applyExpandState();
  // Mod list operations: the selected mods, an operation's events turned into model
  // updates, the context menu, drops, and the live deploy kept in step.
  QStringList selectedMods() const;
  void applyModEvents(const rust::Vec<strata::ModEvent>& events);
  void rebuildModList();
  void onModItemChanged(QStandardItem* item);
  void onModDrop(const QModelIndex& parent, int row, bool belowExpandedSeparator);
  // Where a drop lands, worked out at the drop (the view's indexes change afterwards):
  // by ascending priority the core's tree position, in a flat list a priority.
  struct DropTarget
  {
    bool nested = true;
    QString parent;          // the separator dropped into ("" = the top level)
    qint64 sourceRow = -1;   // -1: onto the separator itself
    bool below       = false;
    qint64 priority  = -1;   // flat list: -1 = the end
  };
  std::optional<DropTarget> dropTarget(const QModelIndex& parent, int row, bool belowExpandedSeparator);
  rust::Vec<strata::ModEvent> applyDrop(strata::Session& s, const QStringList& names, const DropTarget& t);
  // A drop from outside the list (ModListDropInfo): an archive is installed, a folder copied
  // into a new mod, a download installed; then moved where it was dropped.
  void onForeignDrop(const QList<QUrl>& urls, const QString& download, const QModelIndex& parent,
                     int row, bool belowExpandedSeparator);
  void copyFolderToMod(const QString& folder, const DropTarget& target);
  void installDropped(const QString& archive, const DropTarget& target);
  void showModContextMenu(const QPoint& pos);
  void addOverwriteActions(QMenu& menu);
  void addSelectionActions(QMenu& menu, const QStringList& names);
  void addModActions(QMenu& menu, const QModelIndex& clicked, const QStringList& names);
  // ModListGlobalContextMenu: install / create at the clicked row, collapse, enable or
  // disable all, refresh (the list options button, the empty part of the list, "All Mods").
  void fillGlobalModMenu(QMenu& menu, const QModelIndex& clicked);
  // Asks (the delete confirm), then removes the mods to the trash or for good.
  void removeMods(const QStringList& names);
  // The Sub-Separator level dialog of a separator (Ctrl+Shift+Enter).
  void editSeparatorLevel(const QString& name);
  void refreshDeploy();
  // The conflict index: built on a worker, updated there after a change (the index moves
  // to the worker and back), then shown in the Conflicts and Flags columns.
  void buildConflicts();
  void updateConflicts();
  void applyAllConflicts();
  void updateMarkers();
  // The filter panel's criteria applied to the rows (strata_ui::filter on every row's facts).
  void applyCriteria();
  template <class Op>
  void modOperation(const char* what, Op op);
  void updatePluginCounter();
  void switchProfile(int comboIndex);
  // Switches to `name`: selects it in the instance and shows it. False on an error.
  bool selectProfile(const QString& name);
  // The profile row's backup buttons (mods: modlist.txt; else the load order files).
  void backupOrRestore(bool mods, bool restore);
  // Shows `name` (already selected in the instance): a new session, every list again.
  bool showProfile(const QString& name);
  void showError(const QString& message);
  void onModSortChanged(int column, Qt::SortOrder order);
  void openFolder(const QString& path);

  std::unique_ptr<Ui::MainWindow> ui;
  Style& m_style;
  QString m_appliedStyle;  // the style name applyInstanceStyle last applied
  std::optional<rust::Box<strata::Session>> m_session;
  std::function<void()> m_installActions;  // install area: re-reads "auto-enable" (sync)
  class QLocalServer* m_linkServer = nullptr;
  strata::DownloadQueue* m_downloadQueue = nullptr;  // install area: nxm downloads (owned by the view)
  // The install area's context: the instance, a log line, refresh, "Configure Mod?".
  strata::DownloadContext downloadContext();

  // Two models: the next list is built in the one the proxy doesn't show, then swapped in
  // (filling a shown model row by row costs a proxy and view update per row: 240 ms for
  // a 1,365-row list, against about 10 ms).
  QStandardItemModel m_modModels[2];
  QStandardItemModel* m_modModel = &m_modModels[0];
  ModListProxy m_modProxy;
  bool m_modNested = true;
  modlist::RollUps m_rollUps;  // what collapsed separators show (the delegates read it)
  // What the last read of the mod list brought back (from the worker).
  struct ModListData
  {
    rust::Vec<strata::ModRow> rows;
    bool alwaysShowSize = false;
    int indentation     = 0;
    bool displayForeign = true;
    bool invertAlt      = false;  // [Plugins] Sub-Separator\invertAltModifierWhenSeparatorIsCollapsed
    modlist::RollUps rollUps;     // [Settings] collapsible_separators_icons_* / conflicts_*
    QString game;
    std::optional<QStringList> expanded;  // [Widgets] MainWindow_modList_index
    QString error;
    qint64 readMs = 0;
    // the worker's session, handed over to the window
    std::shared_ptr<rust::Box<strata::Session>> session;
  };
  std::shared_ptr<ModListData> m_modData;
  QHash<QString, QStandardItem*> m_modByName;  // name items of the shown model
  bool m_updatingMods = false;                 // the window itself changes the model
  std::shared_ptr<rust::Box<strata::Conflicts>> m_conflicts;
  bool m_conflictsBusy  = false;
  bool m_conflictsAgain = false;
  int m_conflictsGeneration = 0;
  QHash<QString, QColor> m_markers;  // the selection's highlight, by mod name
  bool m_deployRefreshRunning = false;
  bool m_deployRefreshAgain   = false;
  int m_modGeneration = 0;
  QString m_selectAfterLoad;
  int m_oldExecutableIndex = -1;  // the entry before "<Edit...>" was picked  // a mod just installed: selected when the list is back
  // The separators that are open, by name ([Widgets] MainWindow_modList_index, as
  // WidgetSettings::saveTreeExpandState); kept across reloads and filter changes.
  QSet<QString> m_expanded;
  bool m_expandedLoaded = false;
  bool m_applyingExpand = false;
  bool m_fitPlugins     = false;  // the plugin list gets first-start widths when filled
  ColumnFit* m_modFit    = nullptr;  // first-start widths, one per list (columnfit.h)
  ColumnFit* m_pluginFit = nullptr;

  QStandardItemModel m_downloadModel;
  DownloadProxy m_downloadProxy;

  DataTab* m_dataTab       = nullptr;
  DataTab* m_rootTab       = nullptr;
  FilterPanel* m_filterPanel = nullptr;
  PluginsTab* m_pluginsTab   = nullptr;
  LayoutState* m_layout    = nullptr;
  QProcess* m_run          = nullptr;
  QProcess* m_nexusCheck   = nullptr;  // "Check for updates": strata nexus check-updates
  QLabel* m_statusText     = nullptr;
};
