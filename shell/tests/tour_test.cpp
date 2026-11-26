// Not a test: the picture tour (tools/shell-tour.sh). With STRATA_TOUR_INSTANCE set (a copy
// of a real instance) and STRATA_TOUR_DIR, it opens the window there and saves numbered
// pictures of the main window, each tab, the context menus and every dialog, each opened
// the way a user opens it and closed without accepting anything.

#include "tests.h"

#include "modlist/deleteconfirm.h"

#include <QAbstractButton>
#include <QElapsedTimer>
#include <QMenuBar>
#include <QTabWidget>
#include <QTest>
#include <QToolBar>

using namespace drive;

class TourTest : public QObject
{
  Q_OBJECT

  WindowFixture w;
  QString m_dir;
  int m_count = 0;

  void save(QWidget* widget, const QString& name)
  {
    const QString file = QStringLiteral("%1/%2-%3.png").arg(m_dir).arg(++m_count, 2, 10, QChar('0')).arg(name);
    widget->grab().save(file);
    qInfo("tour: %s", qUtf8Printable(file));
  }

  // The next popup menu, pictured and closed (Escape).
  void menuShot(const QString& name, const std::function<void()>& open)
  {
    bool done = false;
    when([] { return QApplication::activePopupWidget() != nullptr; }, [&] {
      QWidget* menu = QApplication::activePopupWidget();
      save(menu, name);
      menu->close();
      done = true;
    });
    open();
    QTRY_VERIFY(done);
  }

  // The next dialog `accept` takes: `inside` (optional, may picture more), then a picture, then closed.
  void dialogShot(const QString& name, std::function<bool(QWidget*)> accept, const std::function<void()>& open,
                  std::function<void(QWidget*)> inside = {}, int settleMs = 400)
  {
    bool done = false;
    whenModal(
        [&, inside, settleMs](QWidget* d) {
          QPointer<QWidget> dialog(d);
          auto start = std::make_shared<QElapsedTimer>();
          start->start();
          // let threads (scans, checks) fill it first
          when([=] { return start->elapsed() > settleMs; }, [&, dialog, inside] {
            if (!dialog) {
              done = true;
              return;
            }
            if (inside) {
              inside(dialog);
            }
            save(dialog, name);
            dialog->close();
            done = true;
          });
        },
        accept, 15000);
    open();
    QTRY_VERIFY_WITH_TIMEOUT(done, 20000);
  }

  QWidget* toolButton(const char* action)
  {
    for (auto* bar : w.window->findChildren<QToolBar*>()) {
      if (QWidget* b = bar->widgetForAction(w.window->findChild<QAction*>(action))) {
        return b;
      }
    }
    return nullptr;
  }

  QModelIndex firstOfKind(int kind) const
  {
    std::function<QModelIndex(const QModelIndex&)> walk = [&](const QModelIndex& parent) -> QModelIndex {
      const auto* m = w.modList()->model();
      for (int r = 0; r < m->rowCount(parent); ++r) {
        const QModelIndex i = m->index(r, 0, parent);
        if (i.data(modlist::KindRole).toInt() == kind) {
          return i;
        }
        if (QModelIndex f = walk(i); f.isValid()) {
          return f;
        }
      }
      return {};
    };
    return walk({});
  }

private slots:
  void initTestCase()
  {
    m_dir = qEnvironmentVariable("STRATA_TOUR_DIR");
    QVERIFY(!m_dir.isEmpty());
    QVERIFY(w.open(qEnvironmentVariable("STRATA_TOUR_INSTANCE")));
    w.window->resize(1600, 1000);
    QTRY_VERIFY_WITH_TIMEOUT(firstOfKind(modlist::Regular).isValid(), 30000);
    QTest::qWait(1500);  // the conflict index and the plugin list
  }

  void cleanupTestCase() { w.close(); }

  void tour()
  {
    auto* tabs = w.child<QTabWidget>("tabWidget");
    tabs->setCurrentIndex(0);
    QTest::qWait(500);
    save(w.window.get(), "main-window");

    for (int i = 0; i < tabs->count(); ++i) {
      tabs->setCurrentIndex(i);
      QTest::qWait(tabs->currentWidget()->objectName() == "dataTab" || tabs->currentWidget()->objectName() == "rootTab"
                       ? 4000 : 800);
      save(w.window.get(), "tab-" + plain(tabs->tabText(i)).toLower());
    }
    tabs->setCurrentIndex(0);

    // the filter panel and a selection with its conflict highlight
    QTest::mouseClick(w.child<QPushButton>("displayCategoriesBtn"), Qt::LeftButton);
    const QModelIndex mod = firstOfKind(modlist::Regular);
    clickRow(w.modList(), mod);
    QTest::qWait(500);
    save(w.window.get(), "categories-and-selection");
    QTest::mouseClick(w.child<QPushButton>("displayCategoriesBtn"), Qt::LeftButton);

    // context menus
    menuShot("menu-mod", [&] { contextMenu(w.modList(), mod); });
    const QModelIndex separator = firstOfKind(modlist::Separator);
    if (separator.isValid()) {
      clickRow(w.modList(), separator);
      menuShot("menu-separator", [&] { contextMenu(w.modList(), separator); });
    }
    const QModelIndex overwrite = firstOfKind(modlist::Overwrite);
    menuShot("menu-overwrite", [&] { contextMenu(w.modList(), overwrite); });
    menuShot("menu-list-options", [&] { QTest::mouseClick(w.child<QPushButton>("listOptionsBtn"), Qt::LeftButton); });
    menuShot("menu-folders", [&] { QTest::mouseClick(w.child<QPushButton>("openFolderMenu"), Qt::LeftButton); });
    menuShot("menu-tools", [&] { QTest::mouseClick(toolButton("actionTool"), Qt::LeftButton); });
    menuShot("menu-help", [&] {
      // the Help menu of the menu bar (Leet's slim toolbar has no Help button)
      w.window->findChild<QMenu*>("menuHelp")->popup(w.window->mapToGlobal(QPoint(200, 20)));
    });
    auto* plugins = w.pluginList();
    if (plugins->model()->rowCount() > 2) {
      const QModelIndex p = plugins->model()->index(2, 0);
      clickRow(plugins, p);
      menuShot("menu-plugin", [&] { contextMenu(plugins, p); });
    }
    tabs->setCurrentWidget(w.child<QWidget>("downloadTab"));
    auto* downloads = w.child<QTreeView>("downloadView");
    if (downloads->model()->rowCount() > 0) {
      const QModelIndex d = downloads->model()->index(0, 0);
      clickRow(downloads, d);
      menuShot("menu-download", [&] { contextMenu(downloads, d); });
    }
    tabs->setCurrentIndex(0);

    // dialogs, opened as a user opens them, closed without accepting
    auto action = [&](const char* name) { return [this, name] { w.window->findChild<QAction*>(name)->trigger(); }; };
    dialogShot("dialog-profiles", named("ProfilesDialog"), action("actionAdd_Profile"));
    dialogShot("dialog-executables", named("EditExecutablesDialog"), action("actionModify_Executables"));
    auto* settingsTabs = new int(0);
    for (int t = 0; t < 9; ++t) {
      dialogShot(QStringLiteral("dialog-settings-%1").arg(t), named("SettingsDialog"), action("actionSettings"),
                 [t, settingsTabs](QWidget* d) {
                   auto* tw = d->findChild<QTabWidget*>("tabWidget");
                   *settingsTabs = tw ? tw->count() : 0;
                   if (tw && t < tw->count()) {
                     tw->setCurrentIndex(t);
                   }
                 });
      if (t + 1 >= *settingsTabs) {
        break;
      }
    }
    delete settingsTabs;
    const QString modName = mod.data(modlist::InternalNameRole).toString();
    for (int t = 0; t < 10; ++t) {
      bool more = true;
      dialogShot(QStringLiteral("dialog-modinfo-%1").arg(t), named("ModInfoDialog"),
                 [&] { w.window->openModInfo(modName, t); },
                 [t, &more](QWidget* d) {
                   auto* tw = d->findChild<QTabWidget*>("tabWidget");
                   more = tw && t + 1 < tw->count();
                 });
      if (!more) {
        break;
      }
    }
    dialogShot("dialog-overwrite", named("OverwriteInfoDialog"),
               [&] { doubleClick(w.modList()->viewport(), onRow(w.modList(), firstOfKind(modlist::Overwrite))); });
    dialogShot("dialog-ini-editor", [](QWidget* d) { return d->inherits("strata::modlist::IniEditorDialog"); },
               [&] {
                 whenMenu({"INI Editor"});
                 QTest::mouseClick(toolButton("actionTool"), Qt::LeftButton);
               });
    dialogShot("dialog-problems", [](QWidget* d) { return d->inherits("strata::modlist::ProblemsDialog"); },
               action("actionNotifications"), {}, 6000);
    if (separator.isValid()) {
      clickRow(w.modList(), separator);
      dialogShot("dialog-separator-level", named("SeparatorDialog"),
                 [&] { QTest::keyClick(w.modList(), Qt::Key_Return, Qt::ControlModifier | Qt::ShiftModifier); });
    }
    {
      // shown directly: the instance may remember an answer ([DialogChoices]), and then Delete
      // would ask nothing (and remove the mod from the copy)
      strata::modlist::DeleteConfirmDialog confirm(w.window.get(), tr("Confirm"),
                                                   tr("Remove the following mods?<br><ul><li>%1</li></ul>").arg(modName));
      confirm.show();
      QTest::qWait(200);
      save(&confirm, "dialog-delete-confirm");
    }
    dialogShot("dialog-install-file", [](QWidget* d) { return d->inherits("QFileDialog"); }, action("actionInstallMod"));
    dialogShot("dialog-about", [](QWidget* d) { return d->inherits("QMessageBox"); }, [&] {
      for (QAction* a : w.window->findChild<QAction*>("actionHelp")->menu()->actions()) {
        if (plain(a->text()) == "About") {
          a->trigger();
        }
      }
    });
  }
};

std::unique_ptr<QObject> drive::makeTourTest()
{
  return std::make_unique<TourTest>();
}

#include "tour_test.moc"
