// The trampoline: Rust decides what this run is (docs/ARCHITECTURE.md, "Entry point").
// Only when there is no subcommand does it call back into shellEntry, so the CLI and vfsd
// never initialise Qt.

#include "loglist.h"
#include "mainwindow.h"
#include "strata_bridge/bridge.h"
#include "style.h"

#include <QApplication>
#include <QIcon>
#include <QCoreApplication>
#include <QTimer>

extern "C" int strata_run(int argc, char** argv,
                          int (*shell)(int, char**, const char*, const char*));

namespace
{

// The sheet for a window with no instance: riki when found, else vs15 Dark
// (strata_ui::style::default_style).
QString defaultStyle()
{
  const rust::String name = strata::default_stylesheet("", QCoreApplication::applicationDirPath().toStdString());
  return QString::fromUtf8(name.data(), qsizetype(name.size()));
}

// Test hooks, all read once at start:
//   STRATA_SHELL_SCREENSHOT=<file.png>  save a picture of the window and quit
//   STRATA_SHELL_SCREENSHOT_DELAY=<ms>  when to take it (default 500)
//   STRATA_SHELL_DIALOG=<name>          open a modlist dialog (profiles, executables,
//                                       settings[:tab], delete); with
//   STRATA_SHELL_DIALOG_SCREENSHOT=<png> save a picture of it and quit
//   STRATA_SHELL_TAB=<index>            select that tab after 1.2 s, as a user would
//                                       (proves the layout is saved on change)
//   STRATA_SHELL_HIDE_COLUMN=<n>       hide that mod list column after 1.2 s
//   STRATA_SHELL_TOGGLE=<mod>          click that mod's checkbox after 1.5 s
//   STRATA_SHELL_DROP=<mod>|<separator> drag that mod onto the separator after 1.8 s
//   STRATA_SHELL_SELECT=<mod>          select that mod after 2.5 s (the conflict highlight)
//   STRATA_SHELL_SHOW_COLUMN=<n>       show that mod list column at once
//   STRATA_SHELL_FILTER=<entry>        tick that filter panel entry after 2 s (e.g. "<Active>")
//   STRATA_SHELL_TOGGLE_PLUGIN=<name>  click that plugin's checkbox after 2.5 s
//   STRATA_SHELL_PROFILE=<name>        switch to that profile after 1.5 s
//   STRATA_SHELL_REFRESH=1             File > Refresh after 1.2 s
//   STRATA_SHELL_RUN_DRY=<title>        run that executable with --dry-run after 1.2 s
//   STRATA_SHELL_MODINFO=<mod>          open that mod's information after 1.2 s (conflicts
//   STRATA_SHELL_MODINFO_TAB=<n>        area), on that tab; the screenshot is then the dialog's
//   STRATA_SHELL_PREVIEW=<path>         preview that file below Data after 1.2 s (conflicts area)
// With QT_QPA_PLATFORM=offscreen they check the window without showing it.
void testHooks(MainWindow& window)
{
  if (const QString tab = qEnvironmentVariable("STRATA_SHELL_TAB"); !tab.isEmpty()) {
    QTimer::singleShot(1200, &window, [&window, tab] { window.selectTab(tab.toInt()); });
  }
  if (const QString column = qEnvironmentVariable("STRATA_SHELL_HIDE_COLUMN"); !column.isEmpty()) {
    QTimer::singleShot(1200, &window, [&window, column] { window.hideModColumn(column.toInt()); });
  }
  if (const QString mod = qEnvironmentVariable("STRATA_SHELL_TOGGLE"); !mod.isEmpty()) {
    QTimer::singleShot(1500, &window, [&window, mod] { window.toggleModForTest(mod); });
  }
  if (const QString drop = qEnvironmentVariable("STRATA_SHELL_DROP"); drop.contains('|')) {
    QTimer::singleShot(1800, &window, [&window, drop] {
      window.dropOntoForTest(drop.section('|', 0, 0), drop.section('|', 1));
    });
  }
  if (const QString mod = qEnvironmentVariable("STRATA_SHELL_SELECT"); !mod.isEmpty()) {
    QTimer::singleShot(2500, &window, [&window, mod] { window.selectMod(mod); });
  }
  for (const QString& c : qEnvironmentVariable("STRATA_SHELL_SHOW_COLUMN").split(',', Qt::SkipEmptyParts)) {
    window.showModColumn(c.toInt());
  }
  if (const QString entry = qEnvironmentVariable("STRATA_SHELL_FILTER"); !entry.isEmpty()) {
    QTimer::singleShot(2000, &window, [&window, entry] { window.tickFilterForTest(entry); });
  }
  if (const QString p = qEnvironmentVariable("STRATA_SHELL_TOGGLE_PLUGIN"); !p.isEmpty()) {
    QTimer::singleShot(2500, &window, [&window, p] { window.togglePluginForTest(p); });
  }
  if (const QString p = qEnvironmentVariable("STRATA_SHELL_PROFILE"); !p.isEmpty()) {
    QTimer::singleShot(1500, &window, [&window, p] { window.switchProfileForTest(p); });
  }
  if (qEnvironmentVariableIsSet("STRATA_SHELL_REFRESH")) {
    QTimer::singleShot(1200, &window, [&window] { window.refresh(); });
  }
  if (const QString title = qEnvironmentVariable("STRATA_SHELL_RUN_DRY"); !title.isEmpty()) {
    QTimer::singleShot(1200, &window, [&window, title] { window.runExecutable(title); });
  }
  if (const QString dialog = qEnvironmentVariable("STRATA_SHELL_DIALOG"); !dialog.isEmpty()) {
    QTimer::singleShot(1200, &window, [&window, dialog] { window.openDialog(dialog); });
  }
  if (const QString mod = qEnvironmentVariable("STRATA_SHELL_MODINFO"); !mod.isEmpty()) {
    const int tab = qEnvironmentVariableIntegerValue("STRATA_SHELL_MODINFO_TAB").value_or(-1);
    QTimer::singleShot(1200, &window, [&window, mod, tab] { window.openModInfo(mod, tab); });
  }
  if (const QString path = qEnvironmentVariable("STRATA_SHELL_PREVIEW"); !path.isEmpty()) {
    QTimer::singleShot(1200, &window, [&window, path] { window.previewDataFile(path); });
  }
  if (const QString file = qEnvironmentVariable("STRATA_SHELL_SCREENSHOT"); !file.isEmpty()) {
    const int delay = qEnvironmentVariableIntegerValue("STRATA_SHELL_SCREENSHOT_DELAY").value_or(500);
    QTimer::singleShot(delay, &window, [&window, file] {
      // A dialog open on top (STRATA_SHELL_MODINFO) is what gets pictured.
      if (QWidget* dialog = QApplication::activeModalWidget()) {
        const bool saved = dialog->grab().save(file);
        dialog->close();
        window.close();
        QCoreApplication::exit(saved ? 0 : 1);
        return;
      }
      const bool saved = window.grab().save(file);
      window.close();  // as a user closing it: the layout is flushed
      QCoreApplication::exit(saved ? 0 : 1);
    });
  }
}

int shellEntry(int argc, char** argv, const char* instance, const char* profile)
{
  QApplication app(argc, argv);
  QApplication::setApplicationName("strata");
  QApplication::setApplicationDisplayName("Strata");
  QApplication::setDesktopFileName("strata");  // the Wayland app id: strata.desktop's icon
  QApplication::setWindowIcon(QIcon(":/MO/gui/app_icon"));
  LogList::installMessageHandler();

  Style style;
  MainWindow window(style);

  QString root;
  try {
    const rust::String chosen = strata::choose_instance(instance);
    root = QString::fromUtf8(chosen.data(), qsizetype(chosen.size()));
  } catch (const std::exception& e) {
    // no instance: the window opens empty, with the reason in the log and status bar
    style.apply(qEnvironmentVariable("STRATA_STYLE", defaultStyle()));
    qCritical("%s", e.what());
  }
  bool opened = false;
  if (!root.isEmpty() && !(opened = window.openInstance(root, QString::fromUtf8(profile)))) {
    style.apply(qEnvironmentVariable("STRATA_STYLE", defaultStyle()));
  }
  window.show();
  // MO2 shows its instance manager when there is no instance to open (a first start, or the
  // last one is gone); the test hooks picture the empty window instead
  if (!opened && qEnvironmentVariableIsEmpty("STRATA_SHELL_SCREENSHOT")) {
    QTimer::singleShot(0, &window, [&window] { window.showInstanceManager(); });
  }

  testHooks(window);
  return QApplication::exec();
}

}  // namespace

int main(int argc, char** argv)
{
  return strata_run(argc, argv, shellEntry);
}
