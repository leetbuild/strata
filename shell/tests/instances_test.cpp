// The instance manager (File > Change Game) and the new-instance wizard: listing the
// instances root and the open instance, rename, delete, and creating an instance against a
// fake Steam (its library has the game, its compatibilitytools.d a Proton).

#include "tests.h"

#include <QAction>
#include <QCheckBox>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QInputDialog>
#include <QListView>
#include <QListWidget>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QTest>

#include "strata_bridge/bridge_instances.h"

using namespace drive;

namespace
{

void put(const QString& path, const QByteArray& text = {})
{
  QDir().mkpath(QFileInfo(path).path());
  QFile f(path);
  QVERIFY(f.open(QIODevice::WriteOnly));
  f.write(text);
}

QStringList rows(QWidget* dialog)
{
  auto* list = dialog->findChild<QListView*>("list");
  QStringList out;
  for (int r = 0; r < list->model()->rowCount(); ++r) {
    out << list->model()->index(r, 0).data().toString();
  }
  return out;
}

void selectRow(QWidget* dialog, const QString& name)
{
  auto* list = dialog->findChild<QListView*>("list");
  const QModelIndex index = find(list->model(), Qt::DisplayRole, name);
  QVERIFY2(index.isValid(), qPrintable(name));
  list->selectionModel()->select(index, QItemSelectionModel::ClearAndSelect);
}

QPushButton* button(QWidget* dialog, const char* name)
{
  return dialog->findChild<QPushButton*>(QString::fromLatin1(name));
}

}  // namespace

class InstancesTest : public QObject
{
  Q_OBJECT

  WindowFixture w;
  QString m_root;
  QByteArray m_home;

private slots:
  void initTestCase()
  {
    m_root = stage() + "/instances-root";
    QDir(m_root).removeRecursively();
    makeInstance("instances-root/Other");
    QVERIFY(w.open(makeInstance("instances")));
    strata::instances::set_instances_root(rust::String(m_root.toStdString()));
    // a fake Steam in a fake home: Skyrim SE in its library, one GE-Proton
    m_home            = qgetenv("HOME");
    const QString home = stage() + "/instances-home";
    QDir(home).removeRecursively();
    const QString steam = home + "/.local/share/Steam";
    put(steam + "/steamapps/appmanifest_489830.acf", "\"AppState\"\n{\n\t\"installdir\"\t\t\"Skyrim Special Edition\"\n}\n");
    put(steam + "/steamapps/common/Skyrim Special Edition/SkyrimSE.exe");
    put(steam + "/steamapps/common/Skyrim Special Edition/skse64_loader.exe");
    put(steam + "/compatibilitytools.d/GE-Proton9-99/proton");
    qputenv("HOME", home.toUtf8());
  }

  void cleanupTestCase()
  {
    qputenv("HOME", m_home);
    w.close();
  }

  void theManagerListsRenamesAndDeletes()
  {
    QStringList seen, afterRename, afterDelete;
    bool renamed = false;
    whenModal(
        [&](QWidget* d) {
          seen = rows(d);
          QVERIFY(!button(d, "switchToInstance")->isEnabled());  // the open instance is selected
          selectRow(d, "Other");
          QVERIFY(button(d, "switchToInstance")->isEnabled());
          QCOMPARE(d->findChild<QLineEdit*>("location")->text(), m_root + "/Other");
          // Rename: the name asked in an input box
          whenModalOf<QInputDialog>([&, d](QInputDialog* input) {
            input->setTextValue("Renamed");
            input->accept();
            // then Delete, permanently
            QPointer<QWidget> dialog(d);
            when([=] { return dialog && rows(dialog).contains("Renamed"); }, [&, dialog] {
              renamed     = true;
              afterRename = rows(dialog);
              selectRow(dialog, "Renamed");
              whenModal(
                  [&, dialog](QWidget* confirm) {
                    auto* list = confirm->findChild<QListWidget*>();
                    QCOMPARE(list->count(), 1);
                    QCOMPARE(list->item(0)->checkState(), Qt::Checked);
                    for (auto* b : confirm->findChildren<QPushButton*>()) {
                      if (b->text() == "Delete permanently") {
                        QTest::mouseClick(b, Qt::LeftButton);
                      }
                    }
                    when([=] { return dialog && QApplication::activeModalWidget() == dialog.data() &&
                                      !rows(dialog).contains("Renamed"); },
                         [&, dialog] {
                           afterDelete = rows(dialog);
                           QTest::mouseClick(button(dialog, "close"), Qt::LeftButton);
                         });
                  },
                  named("deleteInstanceDialog"));
              QTest::mouseClick(button(dialog, "deleteInstance"), Qt::LeftButton);
            });
          });
          QTest::mouseClick(button(d, "rename"), Qt::LeftButton);
        },
        named("InstanceManagerDialog"));
    w.window->findChild<QAction*>("actionChange_Game")->trigger();
    QVERIFY2(seen.contains("instances") && seen.contains("Other"), qPrintable(seen.join(", ")));
    QVERIFY(renamed);
    QVERIFY(afterRename.contains("Renamed") && !afterRename.contains("Other"));
    QVERIFY(!QDir(m_root + "/Other").exists());
    QVERIFY(!QDir(m_root + "/Renamed").exists());
    QVERIFY2(!afterDelete.contains("Renamed") && afterDelete.contains("instances"), qPrintable(afterDelete.join(", ")));
  }

  void theWizardCreatesAnInstance()
  {
    const QString folder = m_root + "/Skyrim Special Edition";
    bool reviewed        = false;
    whenModal(
        [&](QWidget* d) {
          auto* next = button(d, "next");
          auto* games = d->findChild<QListWidget*>("games");
          QCOMPARE(games->currentRow(), 0);
          QVERIFY(d->findChild<QLineEdit*>("gameDir")->text().endsWith("steamapps/common/Skyrim Special Edition"));
          QTest::mouseClick(next, Qt::LeftButton);
          QCOMPARE(d->findChild<QLineEdit*>("instanceName")->text(), QString("Skyrim Special Edition"));
          QCOMPARE(d->findChild<QLineEdit*>("location")->text(), folder);
          QTest::mouseClick(next, Qt::LeftButton);
          QCOMPARE(d->findChild<QComboBox*>("proton")->currentText(), QString("GE-Proton9-99"));
          QTest::mouseClick(next, Qt::LeftButton);
          shoot(d, "create-instance-review");
          reviewed = d->findChild<QPlainTextEdit*>("review")->toPlainText().contains("GE-Proton9-99");
          d->findChild<QCheckBox*>("setupPrefix")->setChecked(false);  // no Proton run in a test
          d->findChild<QCheckBox*>("launch")->setChecked(false);       // no restart either
          QTest::mouseClick(next, Qt::LeftButton);                      // Create
          QCOMPARE(next->text(), QString("Close"));
          QTest::mouseClick(next, Qt::LeftButton);
        },
        named("CreateInstanceDialog"));
    w.window->openDialog("create-instance");
    QVERIFY(reviewed);
    QVERIFY(fileText(folder + "/ModOrganizer.ini").contains("gameName=Skyrim Special Edition"));
    QVERIFY(fileText(folder + "/ModOrganizer.ini").contains("GE-Proton9-99"));
    QVERIFY(QFileInfo(folder + "/profiles/Default/modlist.txt").isFile());
  }
};

namespace drive
{
std::unique_ptr<QObject> makeInstancesTest()
{
  return std::make_unique<InstancesTest>();
}
}  // namespace drive

#include "instances_test.moc"
