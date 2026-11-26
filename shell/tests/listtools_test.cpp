// The list options menu's Check for updates, Auto assign categories and Export to csv, and
// Overwrite's Sync to Mods (shell/src/listtools.*, strata_ui::listtools).

#include "tests.h"

#include <QComboBox>
#include <QLabel>
#include <QMessageBox>
#include <QPushButton>
#include <QTest>
#include <QTextEdit>
#include <QToolButton>
#include <QTreeWidget>

using namespace drive;

namespace
{

void write(const QString& path, const QByteArray& text)
{
  QFile f(path);
  QVERIFY(f.open(QIODevice::WriteOnly));
  f.write(text);
}

QTreeWidgetItem* findItem(QTreeWidgetItem* item, const QString& name)
{
  if (item->text(0) == name) {
    return item;
  }
  for (int i = 0; i < item->childCount(); ++i) {
    if (auto* found = findItem(item->child(i), name)) {
      return found;
    }
  }
  return nullptr;
}

}  // namespace

class ListToolsTest : public QObject
{
  Q_OBJECT

  WindowFixture w;

  void listOptions(const QStringList& path)
  {
    whenMenu(path);
    w.child<QPushButton>("listOptionsBtn")->showMenu();
  }

private slots:
  void initTestCase()
  {
    QVERIFY(w.open(makeInstance("listtools")));
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("SKSE").isValid(), 15000);
  }

  void cleanupTestCase() { w.close(); }

  void exportToCsvListsTheMods()
  {
    QString text;
    whenModal([](QWidget* d) { d->findChild<QDialogButtonBox*>()->button(QDialogButtonBox::Ok)->click(); },
              named("exportCsvDialog"));
    whenModal(
        [&text](QWidget* d) {
          text = d->findChild<QTextEdit*>("textEdit")->toPlainText();
          d->findChild<QPushButton*>("closeBtn")->click();
        },
        named("SaveTextAsDialog"));
    listOptions({"Export to csv..."});
    QTRY_VERIFY(!text.isEmpty());
    const QStringList lines = text.split('\n', Qt::SkipEmptyParts);
    QCOMPARE(lines.first(), QString("#Mod_Priority,#Mod_Status,#Mod_Name"));
    QVERIFY2(lines.contains("\"0000\",\"+\",\"Redundant\""), qPrintable(text));
    QVERIFY(text.contains("\"-\",\"Disabled\""));
    QVERIFY(!text.contains("Overwrite"));
  }

  void autoAssignCategoriesUsesTheNexusMap()
  {
    write(w.root + "/categories.dat", "1|Animations|0|0\n7|Patches|0|0\n");
    write(w.root + "/nexuscatmap.dat", "1|Animation|51\n7|Patches|79\n");
    write(w.root + "/mods/SKSE/meta.ini", "[General]\r\nmodid=0\r\nversion=\r\nnexusCategory=79\r\ncategory=\"1,\"\r\n");
    whenModalOf<QMessageBox>([](QMessageBox* m) { m->button(QMessageBox::Yes)->click(); });
    listOptions({"Auto assign categories"});
    QTRY_VERIFY(fileText(w.root + "/mods/SKSE/meta.ini").contains("category=\"7,\""));
  }

  void checkForUpdatesAsksNexusInAChild()
  {
    // the drive tests have no Nexus key (STRATA_SECRET_TOOL=/bin/false): the child says so
    listOptions({"Check for updates"});
    QTRY_VERIFY_WITH_TIMEOUT(w.child<QLabel>("statusText")->text().contains("update check failed"), 20000);
    QVERIFY(w.child<QLabel>("statusText")->text().contains("Nexus"));
  }

  void syncToModsMovesOverwriteFilesBack()
  {
    const QString ow = w.root + "/overwrite";
    QVERIFY(QFile::exists(ow + "/meshes/b.nif"));
    QString target;
    whenModal(
        [&target](QWidget* d) {
          auto* tree = d->findChild<QTreeWidget*>("syncTree");
          QTreeWidgetItem* nif = findItem(tree->topLevelItem(0), "b.nif");
          QTreeWidgetItem* log = findItem(tree->topLevelItem(0), "written.log");
          QVERIFY(nif && log);
          auto* combo = qobject_cast<QComboBox*>(tree->itemWidget(nif, 1));
          target      = combo->currentText();
          QCOMPARE(qobject_cast<QComboBox*>(tree->itemWidget(log, 1))->count(), 1);  // "<don't sync>" only
          d->findChild<QDialogButtonBox*>()->button(QDialogButtonBox::Ok)->click();
        },
        named("SyncOverwriteDialog"));
    whenMenu({"Sync to Mods..."});
    contextMenu(w.modList(), w.mod("Overwrite"));
    QTRY_VERIFY(!QFile::exists(ow + "/meshes/b.nif"));
    QVERIFY2(!target.isEmpty() && target != "<don't sync>", qPrintable(target));
    // the mod's own spelling of the file (Redundant has meshes/B.nif)
    const QDir mesh(w.root + "/mods/" + target + "/meshes");
    const QStringList found = mesh.entryList({"b.nif"}, QDir::Files);  // QDir filters ignore case
    QCOMPARE(found.size(), 1);
    QCOMPARE(fileText(mesh.filePath(found.first())), QString("overwrite wins"));
    QVERIFY(QFile::exists(ow + "/SKSE/Plugins/written.log"));
  }
};

namespace drive
{
std::unique_ptr<QObject> makeListToolsTest()
{
  return std::make_unique<ListToolsTest>();
}
}  // namespace drive

#include "listtools_test.moc"
