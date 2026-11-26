// The Data tab (folders opened on demand, filter, conflicts only, preview by double click)
// and the mod info dialog's Conflicts tab, driven as a user does.

#include "tests.h"

#include <QCheckBox>
#include <QCoreApplication>
#include <QDir>
#include <QProcess>
#include <QPlainTextEdit>
#include <QTabWidget>
#include <QTest>

using namespace drive;

class DataTest : public QObject
{
  Q_OBJECT

  WindowFixture w;

  QTreeView* data() const { return w.child<QTreeView>("dataTree"); }
  QModelIndex entry(const QString& name, const QModelIndex& parent = {}) const
  {
    const auto* m = data()->model();
    for (int r = 0; r < m->rowCount(parent); ++r) {
      if (m->index(r, 0, parent).data().toString() == name) {
        return m->index(r, 0, parent);
      }
    }
    return {};
  }

private slots:
  void initTestCase()
  {
    // a mod whose only texture is inside its BSA (packed by Strata's own pack-bsa, loose
    // files hidden), loaded by its enabled plugin
    const QString root = makeInstance("data");
    const QString mod  = root + "/mods/Packed";
    QDir().mkpath(mod + "/textures/packed");
    QVERIFY(QFile::copy(root + "/mods/Plugins/a.esp", mod + "/packed.esp"));
    QFile tex(mod + "/textures/packed/only.dds");
    QVERIFY(tex.open(QIODevice::WriteOnly));
    tex.write("packed");
    tex.close();
    QFile meta(mod + "/meta.ini");
    QVERIFY(meta.open(QIODevice::WriteOnly));
    meta.write("[General]\r\nmodid=0\r\n");
    meta.close();
    QFile list(root + "/profiles/Default/modlist.txt");
    QVERIFY(list.open(QIODevice::Append));
    list.write("+Packed\r\n");
    list.close();
    QFile plugins(root + "/profiles/Default/plugins.txt");
    QVERIFY(plugins.open(QIODevice::Append));
    plugins.write("*packed.esp\r\n");
    plugins.close();
    QProcess pack;
    pack.start(QCoreApplication::applicationFilePath(), {"pack-bsa", "Packed", "--hide-loose", "-i", root});
    QVERIFY(pack.waitForFinished(60000));
    QVERIFY2(pack.exitCode() == 0, pack.readAllStandardError().constData());
    QVERIFY(QFile::exists(mod + "/packed.bsa"));
    QVERIFY(!QFile::exists(mod + "/textures/packed/only.dds"));
    QVERIFY(w.open(root));
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("SKSE").isValid(), 15000);
    w.child<QTabWidget>("tabWidget")->setCurrentWidget(w.child<QWidget>("dataTab"));
    QTRY_VERIFY_WITH_TIMEOUT(entry("SKSE").isValid(), 15000);
  }

  void cleanupTestCase() { w.close(); }

  void aFolderOpensWithTheKeyboardAndAFilePreviewsOnDoubleClick()
  {
    data()->setFocus();
    data()->setCurrentIndex(entry("SKSE"));
    QTest::keyClick(data(), Qt::Key_Right);
    QTRY_VERIFY(entry("Plugins", entry("SKSE")).isValid());
    data()->setCurrentIndex(entry("Plugins", entry("SKSE")));
    QTest::keyClick(data(), Qt::Key_Right);
    QTRY_VERIFY(entry("p.ini", entry("Plugins", entry("SKSE"))).isValid());
    const QModelIndex ini = entry("p.ini", entry("Plugins", entry("SKSE")));
    QCOMPARE(ini.siblingAtColumn(1).data().toString(), QString("Plugins"));  // the origin column
    QString text;
    whenModal(
        [&text](QWidget* d) {
          if (auto* edit = d->findChild<QPlainTextEdit*>()) {
            text = edit->toPlainText();
          }
          d->close();
        },
        named("PreviewDialog"));
    doubleClick(data()->viewport(), onRow(data(), ini));
    QTRY_COMPARE(text, QString("Plugins:SKSE/Plugins/p.ini"));
  }

  void theFilterAndConflictsOnlyNarrowTheTree()
  {
    auto* filter = w.child<QLineEdit>("dataTabFilter");
    filter->setFocus();
    QTest::keyClicks(filter, "iron");
    // the matches with their folders (both cases of Case A / Case B fold into one path)
    QTRY_VERIFY(entry("textures").isValid());
    QVERIFY(!entry("SKSE").isValid());
    filter->clear();
    QTRY_VERIFY(entry("SKSE").isValid());
    clickBox(w.child<QCheckBox>("dataTabShowOnlyConflicts"));
    QTRY_VERIFY(!entry("Video").isValid());   // the game's own file, no conflict
    QVERIFY(entry("meshes").isValid());       // a.nif in Case A, Case B and Redundant
    clickBox(w.child<QCheckBox>("dataTabShowOnlyConflicts"));
    QTRY_VERIFY(entry("Video").isValid());
  }

  void theArchivesBoxListsFilesInsideLoadedArchives()
  {
    clickBox(w.child<QCheckBox>("dataTabShowFromArchives"));
    // a new scan with the archives' file lists
    QTRY_VERIFY_WITH_TIMEOUT(entry("textures").isValid(), 15000);
    data()->setFocus();
    data()->setCurrentIndex(entry("textures"));
    QTest::keyClick(data(), Qt::Key_Right);
    QTRY_VERIFY(entry("packed", entry("textures")).isValid());
    data()->setCurrentIndex(entry("packed", entry("textures")));
    QTest::keyClick(data(), Qt::Key_Right);
    QTRY_VERIFY(entry("only.dds", entry("packed", entry("textures"))).isValid());
    const QModelIndex only = entry("only.dds", entry("packed", entry("textures")));
    QCOMPARE(only.siblingAtColumn(1).data().toString(), QString("Packed (packed.bsa)"));
    clickBox(w.child<QCheckBox>("dataTabShowFromArchives"));
    QTRY_VERIFY(!entry("packed", entry("textures")).isValid());
  }

  void aPluginWithAnArchiveShowsItsIcon()
  {
    w.child<QTabWidget>("tabWidget")->setCurrentWidget(w.child<QWidget>("espTab"));
    QTRY_VERIFY_WITH_TIMEOUT(w.plugin("packed.esp").isValid(), 10000);
    const QStringList icons = w.plugin("packed.esp").siblingAtColumn(1).data(modlist::IconsRole).toStringList();
    QVERIFY2(icons.contains(":/MO/gui/archive_conflict_neutral"), qPrintable(icons.join(", ")));
    QVERIFY(w.plugin("packed.esp").data(Qt::ToolTipRole).toString().contains("Loads Archives"));
    w.child<QTabWidget>("tabWidget")->setCurrentWidget(w.child<QWidget>("dataTab"));
  }

  void theModInfoConflictsTabListsWhatAModOverwrites()
  {
    int overwrites = -1, overwritten = -1;
    whenModal(
        [&](QWidget* d) {
          auto* tabs = d->findChild<QTabWidget*>("tabWidget");
          tabs->setCurrentWidget(d->findChild<QWidget*>("tabConflicts"));
          overwrites  = d->findChild<QTreeView*>("overwriteTree")->model()->rowCount();
          overwritten = d->findChild<QTreeView*>("overwrittenTree")->model()->rowCount();
          QTest::mouseClick(d->findChild<QPushButton*>("close"), Qt::LeftButton);
        },
        named("ModInfoDialog"));
    clickRow(w.modList(), w.mod("Case B"));
    whenMenu({"Information..."});
    contextMenu(w.modList(), w.mod("Case B"));
    // Case B wins over Case A (shared.dds, iron.dds, a.nif); overwrite wins its b.nif
    QTRY_VERIFY(overwrites > 0);
    QVERIFY(overwritten > 0);
  }
};

std::unique_ptr<QObject> drive::makeDataTest()
{
  return std::make_unique<DataTest>();
}

#include "data_test.moc"
