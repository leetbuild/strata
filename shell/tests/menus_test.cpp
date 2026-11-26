// The walk through MO2's menus and toolbar (mainwindow.cpp): every action either works or
// is greyed with a tooltip saying why; the list options menu; Visit Nexus; the folder
// menu; Ctrl+Enter.

#include "tests.h"

#include <QInputDialog>
#include <QMenuBar>
#include <QTest>
#include <QToolBar>

using namespace drive;

namespace
{

// Every action of the menu bar (sub-menus included) and the toolbar.
QList<QAction*> allActions(QMainWindow* window)
{
  QList<QAction*> out;
  std::function<void(QMenu*)> walk = [&](QMenu* menu) {
    for (QAction* a : menu->actions()) {
      if (a->isSeparator()) {
        continue;
      }
      out << a;
      if (a->menu()) {
        walk(a->menu());
      }
    }
  };
  for (QAction* a : window->menuBar()->actions()) {
    if (a->menu()) {
      walk(a->menu());
    }
  }
  for (auto* bar : window->findChildren<QToolBar*>()) {
    for (QAction* a : bar->actions()) {
      if (!a->isSeparator()) {
        out << a;
      }
    }
  }
  return out;
}

}  // namespace

class MenusTest : public QObject
{
  Q_OBJECT

  WindowFixture w;

private slots:
  void initTestCase()
  {
    QVERIFY(w.open(makeInstance("menus")));
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("SKSE").isValid(), 15000);
  }

  void cleanupTestCase() { w.close(); }

  void everyActionWorksOrSaysWhyNot()
  {
    QStringList greyed, silent;
    for (QAction* a : allActions(w.window.get())) {
      if (!a->isVisible() || a->isEnabled()) {
        continue;
      }
      const QString tip = a->toolTip();
      if (tip.contains('(') && tip != plain(a->text())) {
        greyed << plain(a->text()) + ": " + tip;
      } else {
        silent << plain(a->text());
      }
    }
    qInfo("greyed: %s", qUtf8Printable(greyed.join(" | ")));
    QVERIFY2(silent.isEmpty(), qPrintable("greyed without a reason: " + silent.join(", ")));
    // and the buttons around the lists
    for (const char* name : {"listOptionsBtn", "linkButton", "sortButton", "restoreButton", "saveButton",
                             "restoreModsButton", "saveModsButton", "openFolderMenu", "startButton"}) {
      auto* b = w.child<QPushButton>(name);
      QVERIFY2(b, name);
      QVERIFY2(b->isEnabled() || !b->toolTip().isEmpty(), name);
    }
  }

  void visitNexusOpensTheGamesPage()
  {
    openedUrls().clear();
    w.window->findChild<QAction*>("actionNexus")->trigger();
    QCOMPARE(openedUrls().value(0), QUrl("https://www.nexusmods.com/skyrimspecialedition"));
  }

  void theFolderMenuOpensTheModsFolder()
  {
    openedUrls().clear();
    whenMenu({"Open Mods folder"});
    QTest::mouseClick(w.child<QPushButton>("openFolderMenu"), Qt::LeftButton);
    QTRY_COMPARE(openedUrls().value(0), QUrl::fromLocalFile(w.root + "/mods"));
  }

  void ctrlEnterOpensTheModsOwnFolder()
  {
    openedUrls().clear();
    clickRow(w.modList(), w.mod("ENB"));
    QTest::keyClick(w.modList(), Qt::Key_Return, Qt::ControlModifier);
    QCOMPARE(openedUrls().value(0), QUrl::fromLocalFile(w.root + "/mods/ENB"));
  }

  void theListOptionsMenuCreatesAnEmptyMod()
  {
    shoot(w.window.get(), "main-window");
    whenMenu({"Create empty mod"});
    whenModalOf<QInputDialog>([](QInputDialog* d) {
      QTest::keyClicks(d->findChild<QLineEdit*>(), "Made Empty");
      clickButton(d, QDialogButtonBox::Ok);
    });
    QTest::mouseClick(w.child<QPushButton>("listOptionsBtn"), Qt::LeftButton);
    QTRY_VERIFY(w.mod("Made Empty").isValid());
    QVERIFY(QFile::exists(w.root + "/mods/Made Empty/meta.ini"));
    QCOMPARE(w.modOrder().last(), QString("Made Empty"));  // on top of the list: the highest priority
  }

  void disableAllMatchingModsTakesOnlyWhatTheFilterShows()
  {
    auto* edit = w.child<QLineEdit>("modFilterEdit");
    edit->setFocus();
    QTest::keyClicks(edit, "case");
    QTRY_VERIFY(!w.mod("ENB").isValid());
    whenMenu({"Disable all matching mods"});
    QTest::mouseClick(w.child<QPushButton>("listOptionsBtn"), Qt::LeftButton);
    QTRY_VERIFY(listLines(w.profileFile("modlist.txt")).contains("-Case A"));
    QVERIFY(listLines(w.profileFile("modlist.txt")).contains("-Case B"));
    QVERIFY(listLines(w.profileFile("modlist.txt")).contains("+ENB"));
    edit->clear();
    QTRY_VERIFY(w.mod("ENB").isValid());
    // every separator closed (the list options' Collapse all), so the list has an empty part
    whenMenu({"Collapse all"});
    QTest::mouseClick(w.child<QPushButton>("listOptionsBtn"), Qt::LeftButton);
    QTRY_VERIFY(!w.modList()->isExpanded(w.mod("Top_separator")));
    QVERIFY(!w.modList()->indexAt(QPoint(5, w.modList()->viewport()->height() - 5)).isValid());
    whenMenu({"Enable all"});
    contextMenu(w.modList(), QModelIndex());  // the empty part of the list: the same menu
    QTRY_VERIFY(listLines(w.profileFile("modlist.txt")).contains("+Case A"));
    QVERIFY(listLines(w.profileFile("modlist.txt")).contains("+Disabled"));
  }

  void theToolMenuOpensTheIniEditor()
  {
    bool opened = false;
    whenModal([&opened](QWidget* d) {
      opened = true;
      shoot(d, "ini-editor");
      d->close();
    }, [](QWidget* d) { return d->inherits("strata::modlist::IniEditorDialog"); });
    whenMenu({"INI Editor"});
    // the toolbar's Tool Plugins button opens its menu at once
    for (auto* bar : w.window->findChildren<QToolBar*>()) {
      if (QWidget* b = bar->widgetForAction(w.window->findChild<QAction*>("actionTool"))) {
        QTest::mouseClick(b, Qt::LeftButton);
      }
    }
    QTRY_VERIFY(opened);
  }

  void notificationsOpensTheProblemsDialog()
  {
    bool opened = false;
    whenModal([&opened](QWidget* d) {
      opened = true;
      d->close();
    }, [](QWidget* d) { return d->inherits("strata::modlist::ProblemsDialog"); });
    auto* action = w.window->findChild<QAction*>("actionNotifications");
    QVERIFY(action->isEnabled());
    action->trigger();
    QTRY_VERIFY(opened);
  }

  void createEmptyModInsideASeparator()
  {
    clickRow(w.modList(), w.mod("Nest 5_separator"));
    whenMenu({"All Mods", "Create empty mod inside"});
    whenModalOf<QInputDialog>([](QInputDialog* d) {
      QTest::keyClicks(d->findChild<QLineEdit*>(), "Inside Five");
      clickButton(d, QDialogButtonBox::Ok);
    });
    contextMenu(w.modList(), w.mod("Nest 5_separator"));
    QTRY_VERIFY(w.mod("Inside Five").isValid());
    QCOMPARE(w.mod("Inside Five").parent(), w.mod("Nest 5_separator"));
  }
};

std::unique_ptr<QObject> drive::makeMenusTest()
{
  return std::make_unique<MenusTest>();
}

#include "menus_test.moc"
