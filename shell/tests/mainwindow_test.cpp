// The main window driven as a user does it, on a generated instance: check boxes, drags,
// separators opened and closed, the filter box and panel, the context menu, the profile
// combo, the plugin list. Each test checks the files MO2 reads (modlist.txt,
// plugins.txt, loadorder.txt, meta.ini, ModOrganizer.ini).

#include "tests.h"

#include "pluginstab.h"
#include "modlist.h"

#include <QCheckBox>
#include <QDir>
#include <QInputDialog>
#include <QSpinBox>
#include <QTabWidget>
#include <QTreeWidget>
#include <QTest>

using namespace drive;

class MainWindowTest : public QObject
{
  Q_OBJECT

  WindowFixture w;

  QString order() const { return w.modOrder().join(", "); }

private slots:
  void initTestCase()
  {
    const QString root = makeInstance("main");
    // a second profile, as MO2's "Copy" makes it
    QVERIFY(copyTree(root + "/profiles/Default", root + "/profiles/Second"));
    QVERIFY(w.open(root));
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("SKSE").isValid(), 15000);
  }

  void cleanupTestCase() { w.close(); }

  void aCheckboxClickEnablesAndDisablesAMod()
  {
    QVERIFY(listLines(w.profileFile("modlist.txt")).contains("-Disabled"));
    clickCheckBox(w.modList(), w.mod("Disabled"));
    QTRY_VERIFY(listLines(w.profileFile("modlist.txt")).contains("+Disabled"));
    QCOMPARE(w.mod("Disabled").data(Qt::CheckStateRole).toInt(), int(Qt::Checked));
    clickCheckBox(w.modList(), w.mod("Disabled"));
    QTRY_VERIFY(listLines(w.profileFile("modlist.txt")).contains("-Disabled"));
    QCOMPARE(w.mod("Disabled").data(Qt::CheckStateRole).toInt(), int(Qt::Unchecked));
  }

  void aModDraggedOntoASeparatorGoesAfterItsOwnMods()
  {
    // Top_separator's last child is the sub-separator Nest 1 (nested five deep): the flat
    // list's end of Top is inside Nest 5, so SKSE goes after Top's own mods, before Nest 1
    clickRow(w.modList(), w.mod("SKSE"));
    QVERIFY(dragSelectionTo(w.modList(), onRow(w.modList(), w.mod("Top_separator"))));
    QTRY_COMPARE(w.modOrder().indexOf("SKSE"), w.modOrder().indexOf("Nest 1_separator") - 1);
    QCOMPARE(w.mod("SKSE").parent(), w.mod("Top_separator"));
  }

  void aModDraggedAboveARowTakesItsPlace()
  {
    // ascending: Redundant (0), ENB, SKSE copy, Top_separator...
    QCOMPARE(w.modOrder().first(), QString("Redundant"));
    clickRow(w.modList(), w.mod("SKSE copy"));
    QVERIFY(dragSelectionTo(w.modList(), aboveRow(w.modList(), w.mod("Redundant"))));
    QTRY_COMPARE(w.modOrder().first(), QString("SKSE copy"));
    QCOMPARE(w.modOrder().at(1), QString("Redundant"));
  }

  void severalModsDraggedTogether()
  {
    clickRow(w.modList(), w.mod("ENB"));
    clickRow(w.modList(), w.mod("Redundant"), Qt::ControlModifier);
    QCOMPARE(w.modList()->selectionModel()->selectedRows().size(), 2);
    QVERIFY(dragSelectionTo(w.modList(), aboveRow(w.modList(), w.mod("SKSE copy"))));
    QTRY_COMPARE(w.modOrder().mid(0, 3), (QStringList{"Redundant", "ENB", "SKSE copy"}));
  }

  // MO2's roll-ups: a collapsed separator shows the conflict and content icons of every mod
  // inside it (collapsible_separators_icons_*), an open one only its own
  void aCollapsedSeparatorShowsTheIconsOfTheModsInside()
  {
    QTreeView* view = w.modList();
    const QModelIndex top = w.mod("Top_separator");
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("Case A").siblingAtColumn(modlist::ColConflictFlags).data(modlist::IconsRole).toStringList().size() > 0, 10000);
    auto iconCount = [view](const QModelIndex& index, int column) {
      QStyleOptionViewItem option;
      option.rect = QRect(0, 0, 400, 20);
      return view->itemDelegateForColumn(column)->sizeHint(option, index.siblingAtColumn(column)).width() / 20;
    };
    view->expand(top);
    const int open = iconCount(top, modlist::ColConflictFlags);
    view->collapse(top);
    QVERIFY2(iconCount(w.mod("Top_separator"), modlist::ColConflictFlags) > open, "no conflict icons rolled up");
    QVERIFY(iconCount(w.mod("Top_separator"), modlist::ColContent) > 0);
    view->expand(w.mod("Top_separator"));
    QCOMPARE(iconCount(w.mod("Top_separator"), modlist::ColConflictFlags), open);
  }

  void aSeparatorClosedByItsArrowIsSavedClosed()
  {
    QTreeView* view = w.modList();
    const QModelIndex top = w.mod("Top_separator");
    QVERIFY(view->isExpanded(top));
    // indentation 0 (MO2's default): the arrow is drawn left of the name
    const QRect r = view->visualRect(top);
    QTest::mouseClick(view->viewport(), Qt::LeftButton, Qt::NoModifier, QPoint(r.x() - r.height() / 2, r.center().y()));
    QVERIFY(!view->isExpanded(w.mod("Top_separator")));
    // saved by the layout timer under MO2's key, without the closed separator
    QTRY_VERIFY_WITH_TIMEOUT(w.ini().contains("MainWindow_modList_index="), 3000);
    QVERIFY(!w.ini().section("MainWindow_modList_index=", 1).section('\n', 0, 0).contains("Top"));
    // the keyboard opens it again
    view->setCurrentIndex(w.mod("Top_separator"));
    QTest::keyClick(view, Qt::Key_Right);
    QVERIFY(view->isExpanded(w.mod("Top_separator")));
    QTRY_VERIFY_WITH_TIMEOUT(w.ini().section("MainWindow_modList_index=", 1).section('\n', 0, 0).contains("Top"), 3000);
  }

  // MO2's bug: a press on the arrow left QAbstractItemView with an older press position, and a
  // move before the release drag-selected from there (the top of the list after start)
  void aSeparatorArrowPressWithAMoveSelectsNothing()
  {
    QTreeView* view = w.modList();
    view->clearSelection();
    const QModelIndex top = w.mod("Top_separator");
    const bool open = view->isExpanded(top);
    const QRect r = view->visualRect(top);
    const QPoint at(r.x() - r.height() / 2, r.center().y());
    auto send = [&](QEvent::Type type, QPoint p, Qt::MouseButton button, Qt::MouseButtons buttons) {
      QMouseEvent e(type, p, view->viewport()->mapToGlobal(p), button, buttons, Qt::NoModifier);
      QCoreApplication::sendEvent(view->viewport(), &e);
    };
    send(QEvent::MouseButtonPress, at, Qt::LeftButton, Qt::LeftButton);
    for (int dy : {2, 5, 9}) {
      send(QEvent::MouseMove, at + QPoint(1, dy), Qt::NoButton, Qt::LeftButton);
    }
    send(QEvent::MouseButtonRelease, at + QPoint(1, 9), Qt::LeftButton, Qt::NoButton);
    QCOMPARE(view->isExpanded(w.mod("Top_separator")), !open);
    QCOMPARE(view->selectionModel()->selectedRows().size(), 0);
    // put it back for the tests after this one
    send(QEvent::MouseButtonPress, at, Qt::LeftButton, Qt::LeftButton);
    send(QEvent::MouseButtonRelease, at, Qt::LeftButton, Qt::NoButton);
    QCOMPARE(view->isExpanded(w.mod("Top_separator")), open);
  }

  void theFilterBoxHidesWhatDoesNotMatch()
  {
    auto* edit = w.child<QLineEdit>("modFilterEdit");
    edit->setFocus();
    QTest::keyClicks(edit, "case");
    QTRY_VERIFY(!w.mod("ENB").isValid());
    QVERIFY(w.mod("Case A").isValid());
    QVERIFY(w.mod("Case B").isValid());
    QVERIFY(w.mod("Top_separator").isValid());  // a separator stays while something inside matches
    QTest::keyClick(edit, Qt::Key_A, Qt::ControlModifier);
    QTest::keyClick(edit, Qt::Key_Backspace);
    QTRY_VERIFY(w.mod("ENB").isValid());
  }

  void theFilterPanelShowsOnlyActiveMods()
  {
    QTest::mouseClick(w.child<QPushButton>("displayCategoriesBtn"), Qt::LeftButton);
    auto* filters = w.child<QTreeWidget>("filters");
    QVERIFY(filters->isVisible());
    QTreeWidgetItem* active = nullptr;
    for (int i = 0; i < filters->topLevelItemCount(); ++i) {
      if (filters->topLevelItem(i)->text(1) == "<Active>") {
        active = filters->topLevelItem(i);
      }
    }
    QVERIFY2(active, "no <Active> entry in the filter panel");
    const QRect r = filters->visualItemRect(active);
    QTest::mouseClick(filters->viewport(), Qt::LeftButton, Qt::NoModifier, r.center());
    QTRY_VERIFY(!w.mod("Disabled").isValid());
    QVERIFY(w.mod("ENB").isValid());
    // a right click steps back: off, then inverted (only the inactive ones)
    QTest::mouseClick(filters->viewport(), Qt::RightButton, Qt::NoModifier, r.center());
    QTRY_VERIFY(w.mod("Disabled").isValid());
    QVERIFY(w.mod("ENB").isValid());
    QTest::mouseClick(filters->viewport(), Qt::RightButton, Qt::NoModifier, r.center());
    QTRY_VERIFY(!w.mod("ENB").isValid());
    QVERIFY(w.mod("Disabled").isValid());
    QTest::mouseClick(w.child<QPushButton>("clearFiltersButton"), Qt::LeftButton);
    QTRY_VERIFY(w.mod("Disabled").isValid());
    QTest::mouseClick(w.child<QPushButton>("displayCategoriesBtn"), Qt::LeftButton);
  }

  void theContextMenuSendsToTheBottom()
  {
    clickRow(w.modList(), w.mod("Deep"));
    bool found = false;
    whenMenu({"Send to...", "Highest priority"}, &found);
    contextMenu(w.modList(), w.mod("Deep"));
    QVERIFY(found);
    QTRY_COMPARE(w.modOrder().last(), QString("Deep"));  // the bottom of the screen: the highest priority
  }

  void theContextMenuRenamesAMod()
  {
    clickRow(w.modList(), w.mod("Deep"));
    whenMenu({"Rename Mod..."});
    whenModalOf<QInputDialog>([](QInputDialog* d) {
      auto* edit = d->findChild<QLineEdit*>();
      edit->selectAll();
      QTest::keyClicks(edit, "Deeper");
      clickButton(d, QDialogButtonBox::Ok);
    });
    contextMenu(w.modList(), w.mod("Deep"));
    QTRY_VERIFY(w.mod("Deeper").isValid());
    QVERIFY(QDir(w.root + "/mods/Deeper").exists());
    QVERIFY(!QDir(w.root + "/mods/Deep").exists());
    QCOMPARE(w.modOrder().last(), QString("Deeper"));
  }

  void theContextMenuWritesNotes()
  {
    clickRow(w.modList(), w.mod("ENB"));
    whenMenu({"Edit Notes..."});
    whenModalOf<QInputDialog>([](QInputDialog* d) {
      QTest::keyClicks(d->findChild<QLineEdit*>(), "shaders here");
      clickButton(d, QDialogButtonBox::Ok);
    });
    contextMenu(w.modList(), w.mod("ENB"));
    QTRY_VERIFY(fileText(w.root + "/mods/ENB/meta.ini").contains("comments=shaders here"));
    QTRY_COMPARE(w.mod("ENB").data(modlist::CommentsRole).toString(), QString("shaders here"));
  }

  void theContextMenuCreatesASeparator()
  {
    clickRow(w.modList(), w.mod("Redundant"));
    whenMenu({"All Mods", "Create separator above"});
    whenModalOf<QInputDialog>([](QInputDialog* d) {
      QTest::keyClicks(d->findChild<QLineEdit*>(), "Made Here");
      clickButton(d, QDialogButtonBox::Ok);
    });
    contextMenu(w.modList(), w.mod("Redundant"));
    QTRY_VERIFY(w.mod("Made Here_separator").isValid());
    QVERIFY(QDir(w.root + "/mods/Made Here_separator").exists());
    const QStringList o = w.modOrder();
    QCOMPARE(o.indexOf("Made Here_separator") + 1, o.indexOf("Redundant"));  // just above it
  }

  void theProfileComboSwitchesTheProfile()
  {
    auto* combo = w.child<QComboBox>("profileBox");
    QCOMPARE(combo->currentText(), QString("Default"));
    combo->setFocus();
    QTest::keyClick(combo, Qt::Key_Down);  // the next entry, activated as with the keyboard
    QTRY_VERIFY(w.ini().contains("selected_profile=@ByteArray(Second)"));
    QCOMPARE(combo->currentText(), QString("Second"));
    // the list shown is Second's: a click changes Second's modlist.txt only
    QTRY_VERIFY(w.mod("Disabled").isValid());
    const QString before = fileText(w.profileFile("modlist.txt"));
    clickCheckBox(w.modList(), w.mod("Disabled"));
    QTRY_VERIFY(listLines(w.profileFile("modlist.txt", "Second")).contains("+Disabled"));
    QCOMPARE(fileText(w.profileFile("modlist.txt")), before);
    QTest::keyClick(combo, Qt::Key_Up);
    QTRY_VERIFY(w.ini().contains("selected_profile=@ByteArray(Default)"));
    QTRY_COMPARE(w.mod("Disabled").data(Qt::CheckStateRole).toInt(), int(Qt::Unchecked));
  }

  void aPluginCheckboxWritesPluginsTxt()
  {
    w.child<QTabWidget>("tabWidget")->setCurrentWidget(w.child<QWidget>("espTab"));
    QTRY_VERIFY_WITH_TIMEOUT(!w.window->findChild<PluginsTab*>()->loading(), 10000);
    QVERIFY(w.plugin("a.esp").isValid());
    QVERIFY(listLines(w.profileFile("plugins.txt")).contains("*a.esp"));
    clickCheckBox(w.pluginList(), w.plugin("a.esp"));
    QTRY_VERIFY(listLines(w.profileFile("plugins.txt")).contains("a.esp"));
    QVERIFY(!listLines(w.profileFile("plugins.txt")).contains("*a.esp"));
    QTRY_VERIFY(w.plugin("a.esp").isValid());
    clickCheckBox(w.pluginList(), w.plugin("a.esp"));
    QTRY_VERIFY(listLines(w.profileFile("plugins.txt")).contains("*a.esp"));
  }

  void aPluginDraggedAboveAnotherMovesInLoadorderTxt()
  {
    // the later of two plain plugins dragged above the other (gen-instance's d.esp ...
    // i.esp are only listed: there are no such files; case.esp is a master by its flag)
    auto order = [&] { return listLines(w.profileFile("loadorder.txt")); };
    QTRY_VERIFY(!w.window->findChild<PluginsTab*>()->loading());
    QStringList three{"a.esp", "visible.esp"};
    std::sort(three.begin(), three.end(), [&](const QString& a, const QString& b) {
      return order().indexOf(a) < order().indexOf(b);
    });
    const QString first = three.first(), last = three.last();
    QVERIFY(order().indexOf(first) >= 0);
    clickRow(w.pluginList(), w.plugin(last));
    // MO2's form makes this list InternalMove, which takes only a drag that QDrag started in
    // this view (an offscreen test cannot start one); the drop handling is the same
    w.pluginList()->setDragDropMode(QAbstractItemView::DragDrop);
    QVERIFY(dragSelectionTo(w.pluginList(), aboveRow(w.pluginList(), w.plugin(first))));
    w.pluginList()->setDragDropMode(QAbstractItemView::InternalMove);
    QTRY_COMPARE(order().indexOf(last) + 1, order().indexOf(first));
  }

  void enableAllAndDisableAllAskAndKeepThePrimaryPlugins()
  {
    // PluginListContextMenu's "Enable all" / "Disable all": asked first, primary plugins stay
    QTRY_VERIFY(!w.window->findChild<PluginsTab*>()->loading());
    auto lines   = [&] { return listLines(w.profileFile("plugins.txt")); };
    auto checked = [&](const QString& name) {
      return w.plugin(name).siblingAtColumn(0).data(Qt::CheckStateRole).toInt() == Qt::Checked;
    };
    auto choose = [&](const QString& entry, QDialogButtonBox::StandardButton answer) {
      bool found = false, asked = false;
      whenMenu({entry}, &found);
      whenModal([&asked, answer](QWidget* d) {
        asked = true;
        clickButton(d, answer);
      });
      contextMenu(w.pluginList(), w.plugin("a.esp"));
      QTRY_VERIFY(asked);
      QVERIFY(found);
    };
    QVERIFY(lines().contains("*a.esp"));
    choose("Disable all", QDialogButtonBox::No);
    QVERIFY(lines().contains("*a.esp"));  // "No" changes nothing
    choose("Disable all", QDialogButtonBox::Yes);
    QTRY_VERIFY(lines().contains("a.esp"));
    QVERIFY(!lines().join('\n').contains('*'));
    QTRY_VERIFY(!checked("a.esp"));
    QVERIFY(checked("Skyrim.esm"));
    choose("Enable all", QDialogButtonBox::Yes);
    QTRY_VERIFY(lines().contains("*a.esp"));
    QVERIFY(lines().contains("*visible.esp"));
    QTRY_VERIFY(checked("a.esp"));
    QVERIFY(checked("Skyrim.esm"));
  }

  // ---- the list's keys (ModListView::event) and Alt+click

  void spaceTogglesEverySelectedMod()
  {
    w.modList()->setFocus();
    clickRow(w.modList(), w.mod("ENB"));
    clickRow(w.modList(), w.mod("Disabled"), Qt::ControlModifier);
    QTest::keyClick(w.modList(), Qt::Key_Space);
    QTRY_VERIFY(listLines(w.profileFile("modlist.txt")).contains("+Disabled"));
    QVERIFY(listLines(w.profileFile("modlist.txt")).contains("-ENB"));
    QTest::keyClick(w.modList(), Qt::Key_Space);
    QTRY_VERIFY(listLines(w.profileFile("modlist.txt")).contains("-Disabled"));
    QVERIFY(listLines(w.profileFile("modlist.txt")).contains("+ENB"));
  }

  void ctrlDownMovesTheSelectionOneDown()
  {
    const int before = w.modOrder().indexOf("ENB");
    clickRow(w.modList(), w.mod("ENB"));
    QTest::keyClick(w.modList(), Qt::Key_Down, Qt::ControlModifier);
    QTRY_COMPARE(w.modOrder().indexOf("ENB"), before + 1);
    QTest::keyClick(w.modList(), Qt::Key_Up, Qt::ControlModifier);
    QTRY_COMPARE(w.modOrder().indexOf("ENB"), before);
  }

  void deleteAsksAndCancelKeepsTheMod()
  {
    clickRow(w.modList(), w.mod("Hidden"));
    bool asked = false;
    whenModal([&asked](QWidget* d) {
      asked = true;
      clickButton(d, QDialogButtonBox::Cancel);
    });
    QTest::keyClick(w.modList(), Qt::Key_Delete);
    QTRY_VERIFY(asked);
    QVERIFY(QDir(w.root + "/mods/Hidden").exists());
    QVERIFY(w.mod("Hidden").isValid());
  }

  void altClickOnASeparatorSelectsEverythingInside()
  {
    clickRow(w.modList(), w.mod("Nest 4_separator"), Qt::AltModifier);
    const QModelIndexList rows = w.modList()->selectionModel()->selectedRows();
    QStringList names;
    for (const auto& r : rows) {
      names << r.data(modlist::InternalNameRole).toString();
    }
    // Nest 4 holds Nest 5, which holds the mods above it in the list
    QVERIFY2(names.contains("Nest 4_separator") && names.contains("Nest 5_separator"), qPrintable(names.join(", ")));
    QVERIFY2(names.contains("File over folder"), qPrintable(names.join(", ")));
    QVERIFY(!names.contains("ENB"));
  }

  void shiftEnterOnAModClosesItsSeparator()
  {
    clickRow(w.modList(), w.mod("Case A"));
    const QModelIndex parent = w.mod("Case A").parent();
    QVERIFY(parent.isValid());
    QVERIFY(w.modList()->isExpanded(parent));
    QTest::keyClick(w.modList(), Qt::Key_Return, Qt::ShiftModifier);
    QVERIFY(!w.modList()->isExpanded(w.mod("Case A").parent()));
    QCOMPARE(w.modList()->currentIndex().siblingAtColumn(0), w.mod("Case A").parent());
    QTest::keyClick(w.modList(), Qt::Key_Return, Qt::ShiftModifier);  // on the separator: opens it
    QVERIFY(w.modList()->isExpanded(w.mod("Case A").parent()));
  }

  void ctrlShiftEnterSetsASeparatorsLevel()
  {
    QVERIFY(fileText(w.root + "/mods/Nest 2_separator/meta.ini").contains("nest=2"));
    QVERIFY(w.mod("Nest 2_separator").parent().isValid());
    clickRow(w.modList(), w.mod("Nest 2_separator"));
    whenModal(
        [](QWidget* d) {
          QCOMPARE(d->windowTitle(), QString("Nest 2 - Sub-Separator"));
          shoot(d, "separator-level");
          auto* spin = d->findChild<QSpinBox*>();
          QCOMPARE(spin->value(), 2);
          spin->selectAll();
          QTest::keyClicks(spin, "0");
          clickButton(d, QDialogButtonBox::Ok);
        },
        named("SeparatorDialog"));
    QTest::keyClick(w.modList(), Qt::Key_Return, Qt::ControlModifier | Qt::ShiftModifier);
    QTRY_VERIFY(fileText(w.root + "/mods/Nest 2_separator/meta.ini").contains("nest=0"));
    QTRY_VERIFY(w.mod("Nest 2_separator").isValid() && !w.mod("Nest 2_separator").parent().isValid());
  }
};

std::unique_ptr<QObject> drive::makeMainWindowTest()
{
  return std::make_unique<MainWindowTest>();
}

#include "mainwindow_test.moc"
