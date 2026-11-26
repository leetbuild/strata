// The Sort button's LOOT dialog (shell/src/loot/): asked first, LOOT runs on a masterlist made
// here (offline: STRATA_LOOT_OFFLINE), the dialog shows MO2's report (general messages, plugin
// messages, dirty info, incompatibilities, missing masters, whether the order changed), and the
// plugin list gets the report's icons and tooltips.

#include "loot/lootdialog.h"
#include "pluginstab.h"
#include "tests.h"

#include <QLabel>
#include <QPlainTextEdit>
#include <QProgressBar>
#include <QTabWidget>
#include <QTextBrowser>
#include <QToolButton>

using namespace drive;

namespace
{

// CRC-32 (IEEE), as LOOT matches dirty info to a plugin.
quint32 crc32(const QByteArray& bytes)
{
  quint32 crc = 0xFFFFFFFFu;
  for (char c : bytes) {
    crc ^= quint8(c);
    for (int i = 0; i < 8; ++i) {
      crc = (crc & 1) ? (crc >> 1) ^ 0xEDB88320u : crc >> 1;
    }
  }
  return ~crc;
}

}  // namespace

class LootTest : public QObject
{
  Q_OBJECT

  WindowFixture w;
  QString instance;

  PluginsTab* tab() const { return w.window->findChild<PluginsTab*>(); }
  QStringList order() const { return listLines(w.profileFile("loadorder.txt")); }

private slots:
  void initTestCase()
  {
    instance = makeInstance("loot");
    qputenv("STRATA_LOOT_OFFLINE", "1");
    QFile a(instance + "/mods/Plugins/a.esp");
    QVERIFY(a.open(QIODevice::ReadOnly));
    const QString crc = QString::number(crc32(a.readAll()), 16).rightJustified(8, '0');
    const QString dir = stage() + "/data/strata/loot/skyrimse";
    QDir().mkpath(dir);
    QFile masterlist(dir + "/masterlist.yaml");
    QVERIFY(masterlist.open(QIODevice::WriteOnly));
    masterlist.write(QString("globals:\n"
                             "  - type: warn\n"
                             "    content: 'A general warning from the test masterlist.'\n"
                             "plugins:\n"
                             "  - name: a.esp\n"
                             "    after: [visible.esp]\n"
                             "    inc: [visible.esp]\n"
                             "    msg:\n"
                             "      - type: say\n"
                             "        content: 'A note about a.esp.'\n"
                             "    dirty:\n"
                             "      - crc: 0x%1\n"
                             "        util: 'SSEEdit v4.0.4'\n"
                             "        itm: 3\n"
                             "        udr: 2\n"
                             "        nav: 1\n"
                             "  - name: c.esl\n"
                             "    msg:\n"
                             "      - type: say\n"
                             "        content: 'Not shown: c.esl is disabled.'\n")
                         .arg(crc)
                         .toUtf8());
    masterlist.close();
    QVERIFY(w.open(instance));
    w.child<QTabWidget>("tabWidget")->setCurrentWidget(w.child<QWidget>("espTab"));
    QTRY_VERIFY_WITH_TIMEOUT(!tab()->loading(), 10000);
    // a change writes the profile's lists in Strata's order (libloot must read the same)
    clickCheckBox(w.pluginList(), w.plugin("a.esp"));
    QTRY_VERIFY(!listLines(w.profileFile("plugins.txt")).contains("*a.esp"));
    QTRY_VERIFY(w.plugin("a.esp").isValid());
    clickCheckBox(w.pluginList(), w.plugin("a.esp"));
    QTRY_VERIFY(listLines(w.profileFile("plugins.txt")).contains("*a.esp"));
    QTRY_VERIFY(!tab()->loading());
  }

  void cleanupTestCase()
  {
    w.close();
    qunsetenv("STRATA_LOOT_OFFLINE");
  }

  void noToTheQuestionSortsNothing()
  {
    const QStringList before = order();
    bool asked               = false;
    whenModal([&asked](QWidget* d) {
      asked = true;
      QCOMPARE(d->windowTitle(), QString("Sorting plugins"));
      clickButton(d, QDialogButtonBox::No);
    });
    QTest::mouseClick(w.child<QPushButton>("sortButton"), Qt::LeftButton);
    QTRY_VERIFY(asked);
    QCOMPARE(order(), before);
  }

  void sortShowsTheReportAndMarksThePlugins()
  {
    QVERIFY(order().indexOf("a.esp") < order().indexOf("visible.esp"));
    bool shown = false;
    whenModal([](QWidget* d) { clickButton(d, QDialogButtonBox::Yes); },
              titled("Sorting plugins"));
    whenModal(
        [&shown](QWidget* d) {
          auto* dialog = qobject_cast<LootDialog*>(d);
          QVERIFY(dialog);
          QCOMPARE(dialog->windowTitle(), QString("LOOT"));
          QTRY_VERIFY_WITH_TIMEOUT(dialog->finished(), 20000);
          QVERIFY2(dialog->succeeded(), qUtf8Printable(dialog->reportMarkdown()));
          const QString text = d->findChild<QTextBrowser*>("report")->toPlainText();
          for (const char* expected :
               {"The load order changed: 2 plugin(s) moved.", "General messages",
                "Warning: A general warning from the test masterlist.", "Plugins", "a.esp",
                "Incompatibilities: visible.esp", "A note about a.esp.",
                "SSEEdit v4.0.4 found 3 ITM record(s), 2 deleted reference(s) and 1 deleted "
                "navmesh(es).",
                "stats: "}) {
            QVERIFY2(text.contains(QString::fromUtf8(expected)),
                     qUtf8Printable(QString("missing \"%1\" in:\n%2").arg(expected, text)));
          }
          QVERIFY2(!text.contains("c.esl is disabled"), qUtf8Printable(text));
          QCOMPARE(d->findChild<QLabel*>("progressText")->text(), QString("Done"));
          auto* bar = d->findChild<QProgressBar*>("progressBar");
          QCOMPARE(bar->value(), bar->maximum());
          // the log, under Details
          auto* details = d->findChild<QToolButton*>("details");
          QTest::mouseClick(details, Qt::LeftButton);
          QVERIFY(d->findChild<QWidget*>("detailsPanel")->isVisible());
          const QString log = d->findChild<QPlainTextEdit*>("output")->toPlainText();
          QVERIFY2(log.contains("[info] reading") && log.contains("[info] sorted in"),
                   qUtf8Printable(log));
          // the profile through the overlay, case-insensitive (strata_ui::loot_view)
          if (QFile::exists("/dev/fuse")) {
            QVERIFY2(log.contains("[info] reading the profile through a private overlay mount"),
                     qUtf8Printable(log));
          }
          QTest::qWait(100);
          shoot(d, "loot-report-details");
          QTest::mouseClick(details, Qt::LeftButton);
          QTest::qWait(100);  // the layout after the panel hid
          shoot(d, "loot-report");
          auto* box = d->findChild<QDialogButtonBox*>("buttons");
          QVERIFY(box->button(QDialogButtonBox::Close));
          shown = true;
          clickButton(d, QDialogButtonBox::Close);
        },
        named("LootDialog"));
    QTest::mouseClick(w.child<QPushButton>("sortButton"), Qt::LeftButton);
    QTRY_VERIFY(shown);
    QTRY_VERIFY(order().indexOf("a.esp") > order().indexOf("visible.esp"));

    // PluginList::addLootReport: icons and tooltip from the report, after the list is read again
    QTRY_VERIFY(!tab()->loading());
    QTRY_VERIFY(w.plugin("a.esp").isValid());
    const QModelIndex flags = w.plugin("a.esp").siblingAtColumn(1);
    const QStringList icons = flags.data(modlist::IconsRole).toStringList();
    QVERIFY2(icons.value(0) == ":/MO/gui/warning", qUtf8Printable(icons.join(' ')));
    QVERIFY2(icons.contains(":/MO/gui/information") && icons.last() == ":/MO/gui/edit_clear",
             qUtf8Printable(icons.join(' ')));
    const QString tip = w.plugin("a.esp").data(Qt::ToolTipRole).toString();
    QVERIFY2(tip.contains("Incompatible with visible.esp") && tip.contains("A note about a.esp.") &&
                 tip.contains("found 3 ITM record(s)"),
             qUtf8Printable(tip));
    const QStringList visibleIcons =
        w.plugin("visible.esp").siblingAtColumn(1).data(modlist::IconsRole).toStringList();
    QVERIFY(!visibleIcons.contains(":/MO/gui/information"));
  }

  void aSecondSortSaysTheOrderIsSorted()
  {
    bool shown = false;
    whenModal([](QWidget* d) { clickButton(d, QDialogButtonBox::Yes); },
              titled("Sorting plugins"));
    whenModal(
        [&shown](QWidget* d) {
          auto* dialog = qobject_cast<LootDialog*>(d);
          QTRY_VERIFY_WITH_TIMEOUT(dialog->finished(), 20000);
          QVERIFY(dialog->reportMarkdown().contains("The load order was already sorted."));
          shown = true;
          clickButton(d, QDialogButtonBox::Close);
        },
        named("LootDialog"));
    QTest::mouseClick(w.child<QPushButton>("sortButton"), Qt::LeftButton);
    QTRY_VERIFY(shown);
  }
};

std::unique_ptr<QObject> drive::makeLootTest()
{
  return std::make_unique<LootTest>();
}

#include "loot_test.moc"
