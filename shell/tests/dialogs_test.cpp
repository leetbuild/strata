// The dialogs that exist, opened from the window as a user opens them (menus, buttons,
// combo entries, double clicks), changed, accepted; then the files they write are checked.

#include "tests.h"

#include "modlist/deleteconfirm.h"

#include <QCheckBox>
#include <functional>
#include <QDir>
#include <QInputDialog>
#include <QListWidget>
#include <QMessageBox>
#include <QPlainTextEdit>
#include <QTabWidget>
#include <QTest>
#include <QTextEdit>
#include <QToolBar>
#include <QToolButton>

using namespace drive;

namespace
{

QPushButton* buttonWithText(QWidget* parent, const QString& text)
{
  for (auto* b : parent->findChildren<QPushButton*>()) {
    if (plain(b->text()) == text) {
      return b;
    }
  }
  return nullptr;
}

// Makes the tab holding `w` the current one (each enclosing tab widget).
void showTabOf(QWidget* w)
{
  for (QWidget* p = w; p && p->parentWidget(); p = p->parentWidget()) {
    QWidget* stack = p->parentWidget();
    if (auto* tabs = qobject_cast<QTabWidget*>(stack->parentWidget()); tabs && tabs->indexOf(p) >= 0) {
      tabs->setCurrentWidget(p);
    }
  }
}

// The toolbar button of an action, clicked as a user clicks it.
void clickToolbar(QMainWindow* window, const char* action)
{
  auto* a = window->findChild<QAction*>(action);
  QVERIFY2(a && a->isEnabled(), action);
  for (auto* bar : window->findChildren<QToolBar*>()) {
    if (QWidget* w = bar->widgetForAction(a)) {
      QTest::mouseClick(w, Qt::LeftButton);
      return;
    }
  }
  a->trigger();  // not on the toolbar: as from its menu
}

}  // namespace

class DialogsTest : public QObject
{
  Q_OBJECT

  WindowFixture w;

private slots:
  void initTestCase()
  {
    QVERIFY(w.open(makeInstance("dialogs")));
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("SKSE").isValid(), 15000);
  }

  void cleanupTestCase() { w.close(); }

  // ---- profiles (the combo's "<Manage...>")

  void theProfilesDialogCreatesAProfile()
  {
    whenModal(
        [](QWidget* d) {
          whenModal(
              [](QWidget* input) {
                QTest::keyClicks(input->findChild<QLineEdit*>("nameEdit"), "Made");
                clickButton(input, QDialogButtonBox::Ok);
              },
              named("ProfileInputDialog"));
          QTest::mouseClick(d->findChild<QPushButton*>("addProfileButton"), Qt::LeftButton);
          // back here once the input dialog closed: the new profile is listed and current
          auto* list = d->findChild<QListWidget*>("profilesList");
          QVERIFY(list->currentItem() && list->currentItem()->text() == "Made");
          QTest::mouseClick(d->findChild<QPushButton*>("close"), Qt::LeftButton);
        },
        named("ProfilesDialog"));
    auto* combo = w.child<QComboBox>("profileBox");
    combo->setFocus();
    QTest::keyClick(combo, Qt::Key_Home);  // "<Manage...>", activated as with the keyboard
    QTRY_VERIFY(QFile::exists(w.profileFile("modlist.txt", "Made")));
    QTRY_VERIFY(combo->findText("Made") > 0);  // the window read the profiles again
    QCOMPARE(combo->currentText(), QString("Default"));
  }

  void theProfilesDialogRenamesAndSelects()
  {
    whenModal(
        [](QWidget* d) {
          auto* list = d->findChild<QListWidget*>("profilesList");
          list->setCurrentItem(list->findItems("Made", Qt::MatchExactly).value(0));
          whenModalOf<QInputDialog>([](QInputDialog* input) {
            auto* edit = input->findChild<QLineEdit*>();
            edit->selectAll();
            QTest::keyClicks(edit, "Renamed");
            clickButton(input, QDialogButtonBox::Ok);
          });
          QTest::mouseClick(d->findChild<QPushButton*>("renameButton"), Qt::LeftButton);
          QVERIFY(list->currentItem() && list->currentItem()->text() == "Renamed");
          QTest::mouseClick(d->findChild<QPushButton*>("select"), Qt::LeftButton);
        },
        named("ProfilesDialog"));
    w.window->findChild<QAction*>("actionAdd_Profile")->trigger();  // Tools > Profiles...
    QTRY_VERIFY(w.ini().contains("selected_profile=@ByteArray(Renamed)"));
    QVERIFY(!QDir(w.root + "/profiles/Made").exists());
    QTRY_COMPARE(w.child<QComboBox>("profileBox")->currentText(), QString("Renamed"));
  }

  void theProfilesDialogTurnsLocalSavesOff()
  {
    QVERIFY(fileText(w.profileFile("settings.ini", "Default")).contains("LocalSaves=true"));
    whenModal(
        [](QWidget* d) {
          auto* list = d->findChild<QListWidget*>("profilesList");
          list->setCurrentItem(list->findItems("Default", Qt::MatchExactly).value(0));
          auto* box = d->findChild<QCheckBox*>("localSavesBox");
          QVERIFY(box->isChecked());
          // "delete the profile's saves?" No: they stay
          whenModalOf<QMessageBox>([](QMessageBox* m) { QTest::mouseClick(m->button(QMessageBox::No), Qt::LeftButton); });
          clickBox(box);
          QTest::mouseClick(d->findChild<QPushButton*>("close"), Qt::LeftButton);
        },
        named("ProfilesDialog"));
    w.window->findChild<QAction*>("actionAdd_Profile")->trigger();
    QTRY_VERIFY(fileText(w.profileFile("settings.ini", "Default")).contains("LocalSaves=false"));
    QVERIFY(QFile::exists(w.profileFile("saves/Save 1.ess", "Default")));
  }

  // ---- executables (the combo's "<Edit...>")

  void theExecutablesDialogAddsAnExecutable()
  {
    auto* exes = w.child<QComboBox>("executablesListBox");
    QCOMPARE(exes->currentText(), QString("SKSE"));
    whenModal(
        [](QWidget* d) {
          auto* add = d->findChild<QToolButton*>("add");
          whenMenu({"Add empty"});
          add->showMenu();  // the button's menu, as a press on its arrow opens it
          auto* title = d->findChild<QLineEdit*>("title");
          QCOMPARE(title->text(), QString("New Executable"));
          title->selectAll();
          QTest::keyClicks(title, "Drive Tool");
          QTest::keyClicks(d->findChild<QLineEdit*>("arguments"), "-x");
          QTest::mouseClick(d->findChild<QDialogButtonBox*>("buttons")->button(QDialogButtonBox::Ok), Qt::LeftButton);
        },
        named("EditExecutablesDialog"));
    exes->setFocus();
    QTest::keyClick(exes, Qt::Key_Home);
    QTRY_VERIFY(w.ini().contains("title=Drive Tool"));
    QVERIFY(w.ini().contains("arguments=-x"));
    QTRY_VERIFY(exes->findText("Drive Tool") > 0);
    QCOMPARE(exes->currentText(), QString("SKSE"));  // the entry before "<Edit...>" again
  }

  // ---- settings (the toolbar button)

  void theSettingsDialogStoresLeetsAutoEnable()
  {
    QVERIFY(!w.ini().contains("auto_enable_install"));
    whenModal(
        [](QWidget* d) {
          auto* box = d->findChild<QCheckBox*>("leetAutoEnable");
          QVERIFY(box);
          showTabOf(box);
          const bool before = box->isChecked();
          clickBox(box);
          QCOMPARE(box->isChecked(), !before);
          QTest::mouseClick(d->findChild<QDialogButtonBox*>("buttonBox")->button(QDialogButtonBox::Ok), Qt::LeftButton);
        },
        named("SettingsDialog"));
    clickToolbar(w.window.get(), "actionSettings");
    QTRY_VERIFY(w.ini().contains("auto_enable_install="));
  }

  // Strata's own: the launch environment, one NAME=value per line, stored and read back
  void theLaunchEnvironmentIsStoredAndReadBack()
  {
    auto edit = [](const QString& text, QString* seen) {
      whenModal(
          [text, seen](QWidget* d) {
            auto* button = d->findChild<QPushButton*>("strataEnvironmentBtn");
            QVERIFY(button);
            showTabOf(button);
            whenModalOf<QInputDialog>([text, seen](QInputDialog* input) {
              if (seen) {
                *seen = input->textValue();
              }
              if (!text.isNull()) {
                input->setTextValue(text);
              }
              input->accept();
            });
            QTest::mouseClick(button, Qt::LeftButton);
            QTest::mouseClick(d->findChild<QDialogButtonBox*>("buttonBox")->button(QDialogButtonBox::Ok), Qt::LeftButton);
          },
          named("SettingsDialog"));
    };
    edit("PROTON_NO_NTSYNC=1\nDXVK_HUD=fps", nullptr);
    clickToolbar(w.window.get(), "actionSettings");
    QTRY_VERIFY(w.ini().contains("strata_environment="));
    QString seen;
    edit(QString(), &seen);
    clickToolbar(w.window.get(), "actionSettings");
    QTRY_COMPARE(seen, QString("PROTON_NO_NTSYNC=1\nDXVK_HUD=fps"));
  }

  // MO2 applies the style chosen in the dialog on OK; it used to wait for the next start
  void aStyleChosenInTheSettingsIsAppliedOnOk()
  {
    const QString before = qApp->styleSheet();
    QString chosen;
    whenModal(
        [&chosen, &before](QWidget* d) {
          auto* box = d->findChild<QComboBox*>("styleBox");
          QVERIFY(box);
          showTabOf(box);
          // any sheet other than the one in use
          for (int i = 1; i < box->count(); ++i) {
            if (!before.endsWith("/" + box->itemData(i).toString())) {
              box->setCurrentIndex(i);
              chosen = box->itemData(i).toString();
              break;
            }
          }
          QTest::mouseClick(d->findChild<QDialogButtonBox*>("buttonBox")->button(QDialogButtonBox::Ok), Qt::LeftButton);
        },
        named("SettingsDialog"));
    clickToolbar(w.window.get(), "actionSettings");
    QVERIFY(!chosen.isEmpty());
    QTRY_VERIFY(w.ini().contains("style=" + chosen));
    QTRY_VERIFY(qApp->styleSheet().endsWith("/" + chosen));
  }

  // ---- mod information (double click), Previous / Next

  void theModInfoDialogSavesNotesAndSteps()
  {
    QString shown;
    whenModal(
        [&shown](QWidget* d) {
          auto* tabs = d->findChild<QTabWidget*>("tabWidget");
          tabs->setCurrentWidget(d->findChild<QWidget*>("tabNotes"));
          auto* comments = d->findChild<QLineEdit*>("comments");
          comments->setFocus();
          QTest::keyClicks(comments, "from the dialog");
          auto* next = d->findChild<QPushButton*>("nextMod");
          QVERIFY(next->isVisible());
          whenModal(
              [&shown](QWidget* d2) {
                shown = d2->windowTitle();
                // the same tab, the other mod
                QCOMPARE(d2->findChild<QTabWidget*>("tabWidget")->currentWidget()->objectName(), QString("tabNotes"));
                QTest::mouseClick(d2->findChild<QPushButton*>("close"), Qt::LeftButton);
              },
              named("ModInfoDialog"));
          QTest::mouseClick(next, Qt::LeftButton);
        },
        named("ModInfoDialog"));
    const QModelIndex enb = w.mod("ENB");
    const QPoint at       = onRow(w.modList(), enb);
    doubleClick(w.modList()->viewport(), at);
    QTRY_VERIFY(fileText(w.root + "/mods/ENB/meta.ini").contains("comments=from the dialog"));
    // Next = the next mod below ENB on screen
    QCOMPARE(shown, w.window->neighbourMod("ENB", 1));
    // an independent look: the regular mods in the order the tree shows them, depth first
    QStringList flat;
    std::function<void(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
      const auto* m = w.modList()->model();
      for (int r = 0; r < m->rowCount(parent); ++r) {
        const QModelIndex i = m->index(r, 0, parent);
        if (i.data(modlist::KindRole).toInt() == modlist::Regular) {
          flat << i.data(modlist::InternalNameRole).toString();
        }
        walk(i);
      }
    };
    walk({});
    QCOMPARE(shown, flat.value(flat.indexOf("ENB") + 1));
    QCOMPARE(w.window->neighbourMod(shown, -1), QString("ENB"));
    QCOMPARE(w.window->neighbourMod(flat.first(), -1), QString());  // no wrap at the top
  }

  // ---- delete confirm (Leet change 8), remembered

  void removingAModAsksAndRemembers()
  {
    clickRow(w.modList(), w.mod("Empty"));
    whenMenu({"Remove Mod..."});
    whenModalOf<strata::modlist::DeleteConfirmDialog>([](strata::modlist::DeleteConfirmDialog* d) {
      clickBox(d->findChild<QCheckBox*>());  // remember
      QTest::mouseClick(buttonWithText(d, "Delete permanently"), Qt::LeftButton);
    });
    contextMenu(w.modList(), w.mod("Empty"));
    QTRY_VERIFY(!w.mod("Empty").isValid());
    QVERIFY(!QDir(w.root + "/mods/Empty").exists());
    QVERIFY(!w.modOrder("Renamed").contains("Empty"));
    QVERIFY(w.ini().contains("[DialogChoices]"));
    // remembered: the next removal asks nothing
    clickRow(w.modList(), w.mod("Redundant"));
    whenMenu({"Remove Mod..."});
    contextMenu(w.modList(), w.mod("Redundant"));
    QTRY_VERIFY(!w.mod("Redundant").isValid());
    QVERIFY(!QDir(w.root + "/mods/Redundant").exists());
  }
};

std::unique_ptr<QObject> drive::makeDialogsTest()
{
  return std::make_unique<DialogsTest>();
}

#include "dialogs_test.moc"
