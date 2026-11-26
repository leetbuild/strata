#pragma once

// The Downloads tab's actions (install area): double click installs; the context menu has
// MO2's DownloadList actions that work offline (install, query info from the file name,
// open file / meta file, show in folder, hide / unhide, mark installed / uninstalled,
// delete to the trash), "Install selected", and "Install under separator" (the Install to
// Separator plugin, built in).
//
// The main window calls attachDownloadActions() once; the rows' file names are read from
// column 0's tool tip (MainWindow::loadDownloads puts the file name there).

#include <QString>

#include <functional>

class QAbstractItemView;
class QAction;
class QWidget;

namespace strata
{

struct DownloadContext
{
  // the instance folder and the profile, read when an action runs
  std::function<QString()> root;
  std::function<QString()> profile;
  // the downloads folder
  std::function<QString()> downloads;
  // a line for the log dock
  std::function<void(const QString&)> log;
  // after something changed: reload the lists (and the mod list after an install)
  std::function<void()> refresh;
  // after an install whose INI tweaks the user wants to configure now: show the mod's
  // information on its INI Files tab (OrganizerCore::doInstall). Unset: nothing happens.
  std::function<void(const QString&)> configureIni;
};

class DownloadQueue;

// Install on double click, the context menu, and the download queue (nxm:// links, pause,
// resume). Returns the queue (owned by the view) so the window can hand it links.
DownloadQueue* attachDownloadActions(QAbstractItemView* view, QWidget* window, DownloadContext context);

// The toolbar / menu actions: "Install Mod..." (MO2's MainWindow::installMod: pick an
// archive, starting in the downloads folder) and the checkable "Auto-enable installed mods"
// (Leet change 11). `sync()` re-reads the auto-enable setting (after an instance switch).
struct InstallActions
{
  std::function<void()> sync;
};
InstallActions attachInstallActions(QAction* installMod, QAction* autoEnable, QWidget* window,
                                    DownloadContext context);

}  // namespace strata
