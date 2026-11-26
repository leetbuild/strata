// What the test classes of strata-shell-test share: instances, the window fixture, the list
// of test classes (drive.cpp runs them in order).
#pragma once

#include "driver.h"
#include "mainwindow.h"
#include "modlist.h"
#include "style.h"

#include <QLineEdit>

#include <functional>
#include <memory>
#include <vector>

namespace drive
{

QString repo();
QString stage();
// A fresh tools/gen-instance instance in _stage/shell-drive/<name>.
QString makeInstance(const QString& name);
bool copyTree(const QString& from, const QString& to);

// Every URL the window asked the desktop to open (a browser, a file manager): the tests
// catch them instead (QDesktopServices::setUrlHandler for file, http and https).
QList<QUrl>& openedUrls();

std::unique_ptr<QObject> makeMainWindowTest();
std::unique_ptr<QObject> makeMenusTest();
std::unique_ptr<QObject> makeToolsTest();
std::unique_ptr<QObject> makeTourTest();
std::unique_ptr<QObject> makeDialogsTest();
std::unique_ptr<QObject> makeDropsTest();
std::unique_ptr<QObject> makeDataTest();
std::unique_ptr<QObject> makeOverwriteTest();
std::unique_ptr<QObject> makeColumnsTest();
std::unique_ptr<QObject> makeLootTest();
std::unique_ptr<QObject> makeInstancesTest();
std::unique_ptr<QObject> makeListToolsTest();

inline std::vector<std::function<std::unique_ptr<QObject>()>> testClasses()
{
  if (qEnvironmentVariableIsSet("STRATA_TOUR_INSTANCE")) {
    return {makeTourTest};  // tools/shell-tour.sh: the pictures only
  }
  return {
      makeMainWindowTest, makeDropsTest, makeDialogsTest, makeDataTest, makeOverwriteTest, makeMenusTest, makeToolsTest,
      makeColumnsTest, makeLootTest, makeInstancesTest, makeListToolsTest,
  };
}

// The main window on an instance, shown offscreen at a fixed size.
class WindowFixture
{
public:
  bool open(const QString& instance, const QString& profile = {})
  {
    root   = instance;
    style  = std::make_unique<Style>();
    window = std::make_unique<MainWindow>(*style);
    if (!window->openInstance(root, profile)) {
      return false;
    }
    window->resize(1500, 900);
    window->show();
    return true;
  }
  void close()
  {
    if (window) {
      window->close();
    }
    window.reset();
    style.reset();
  }

  template <class T = QWidget>
  T* child(const char* name) const
  {
    return window->findChild<T*>(QString::fromLatin1(name));
  }
  QTreeView* modList() const { return child<QTreeView>("modList"); }
  QTreeView* pluginList() const { return child<QTreeView>("espList"); }
  QModelIndex mod(const QString& name) const
  {
    return find(modList()->model(), modlist::InternalNameRole, name);
  }
  QModelIndex plugin(const QString& name) const
  {
    return find(pluginList()->model(), Qt::DisplayRole, name);
  }
  QString profileFile(const QString& file, const QString& profile = "Default") const
  {
    return root + "/profiles/" + profile + "/" + file;
  }
  // modlist.txt's mods by ascending priority, without the +/- (foreign "*" lines left out).
  QStringList modOrder(const QString& profile = "Default") const
  {
    QStringList out;
    for (const QString& line : listLines(profileFile("modlist.txt", profile))) {
      if (!line.startsWith('*')) {
        out.prepend(line.mid(1));
      }
    }
    return out;
  }
  QString ini() const { return fileText(root + "/ModOrganizer.ini"); }

  QString root;
  std::unique_ptr<Style> style;
  std::unique_ptr<MainWindow> window;
};

}  // namespace drive
