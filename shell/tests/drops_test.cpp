// Drops from outside onto the mod list (MO2's ModListDropInfo): an archive is installed
// through the installer dialogs (quick install, FOMOD), a folder becomes a new mod; each
// lands where it was dropped.

#include "tests.h"
#include "strata_bridge/bridge_install.h"

#include <QAbstractButton>
#include <QCheckBox>
#include <QTabWidget>
#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QLocalSocket>
#include <QInputDialog>
#include <QLabel>
#include <QListWidget>
#include <QProcess>
#include <QSignalSpy>
#include <QStatusBar>
#include <QTest>

using namespace drive;

namespace
{

void put(const QString& path, const QByteArray& text)
{
  QDir().mkpath(QFileInfo(path).path());
  QFile f(path);
  QVERIFY(f.open(QIODevice::WriteOnly));
  f.write(text);
}

// zip -r <archive> . in `folder`
bool zipFolder(const QString& folder, const QString& archive)
{
  QFile::remove(archive);
  QProcess p;
  p.setWorkingDirectory(folder);
  p.start("zip", {"-qr", archive, "."});
  return p.waitForFinished(30000) && p.exitCode() == 0;
}

std::unique_ptr<QMimeData> urls(const QString& path)
{
  auto mime = std::make_unique<QMimeData>();
  mime->setUrls({QUrl::fromLocalFile(path)});
  return mime;
}

// one install step, one group: Red or Blue
const char* ModuleConfig =
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
    "<config xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:noNamespaceSchemaLocation=\"http://qconsulting.ca/fo3/ModConfig5.0.xsd\">\n"
    "  <moduleName>Drive Fomod</moduleName>\n"
    "  <installSteps order=\"Explicit\">\n"
    "    <installStep name=\"Pick\">\n"
    "      <optionalFileGroups order=\"Explicit\">\n"
    "        <group name=\"Colour\" type=\"SelectExactlyOne\">\n"
    "          <plugins order=\"Explicit\">\n"
    "            <plugin name=\"Red\"><description>red</description><files><folder source=\"red\" destination=\"\" /></files><typeDescriptor><type name=\"Optional\"/></typeDescriptor></plugin>\n"
    "            <plugin name=\"Blue\"><description>blue</description><files><folder source=\"blue\" destination=\"\" /></files><typeDescriptor><type name=\"Optional\"/></typeDescriptor></plugin>\n"
    "          </plugins>\n"
    "        </group>\n"
    "      </optionalFileGroups>\n"
    "    </installStep>\n"
    "  </installSteps>\n"
    "</config>\n";

}  // namespace

class DropsTest : public QObject
{
  Q_OBJECT

  WindowFixture w;
  QString m_outside;

  // Drops an archive (or a download) on the mod list and waits for the real end: installed,
  // moved where it was dropped, the list being read again (MainWindow::dropInstallFinished),
  // then the new row shown. Returns the mod's name ("" when nothing was installed).
  QString dropInstall(QMimeData* mime, QPoint pos)
  {
    QSignalSpy finished(w.window.get(), &MainWindow::dropInstallFinished);
    if (!dropAt(w.modList(), mime, pos, Qt::CopyAction)) {
      return {};
    }
    if (!finished.wait(60000)) {
      qWarning("drive: the install never finished");
      return {};
    }
    const QString name = finished.first().first().toString();
    QDeadlineTimer shown(30000);
    while (!name.isEmpty() && !w.mod(name).isValid() && !shown.hasExpired()) {
      QTest::qWait(20);
    }
    return w.mod(name).isValid() ? name : QString();
  }

private slots:
  void initTestCase()
  {
    m_outside = stage() + "/outside";
    QDir(m_outside).removeRecursively();
    QVERIFY(w.open(makeInstance("drops")));
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("SKSE").isValid(), 15000);
  }

  void cleanupTestCase() { w.close(); }

  void aFolderDroppedOnARowBecomesAModThere()
  {
    put(m_outside + "/Loose Folder/textures/loose.dds", "loose");
    whenModalOf<QInputDialog>([](QInputDialog* d) {
      QCOMPARE(d->textValue(), QString("Loose Folder"));  // MO2 offers the folder's name
      auto* edit = d->findChild<QLineEdit*>();
      edit->selectAll();
      QTest::keyClicks(edit, "From Folder");
      clickButton(d, QDialogButtonBox::Ok);
    });
    auto mime = urls(m_outside + "/Loose Folder");
    QVERIFY(dropAt(w.modList(), mime.get(), aboveRow(w.modList(), w.mod("ENB"))));
    QTRY_VERIFY(w.mod("From Folder").isValid());
    QCOMPARE(fileText(w.root + "/mods/From Folder/textures/loose.dds"), QString("loose"));
    const QStringList o = w.modOrder();
    QCOMPARE(o.indexOf("From Folder") + 1, o.indexOf("ENB"));  // just above the row
    QVERIFY(w.modList()->selectionModel()->isRowSelected(w.mod("From Folder").row(), w.mod("From Folder").parent()));
  }

  void anArchiveDroppedIsInstalledWithTheQuickInstaller()
  {
    put(m_outside + "/simple/textures/simple.dds", "simple");
    const QString archive = m_outside + "/Simple Mod-123-1-0.zip";
    QVERIFY(zipFolder(m_outside + "/simple", archive));
    bool asked = false;
    whenModal(
        [&asked](QWidget* d) {
          asked = true;
          typeName(d->findChild<QComboBox*>("nameCombo"), "Quick One");
          QTest::mouseClick(d->findChild<QPushButton*>("okBtn"), Qt::LeftButton);
        },
        named("SimpleInstallDialog"));
    auto mime = urls(archive);
    QVERIFY(!dropInstall(mime.get(), aboveRow(w.modList(), w.mod("SKSE"))).isEmpty());
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("Quick One").isValid(), 15000);
    QVERIFY(asked);
    QCOMPARE(fileText(w.root + "/mods/Quick One/textures/simple.dds"), QString("simple"));
    QVERIFY(fileText(w.root + "/mods/Quick One/meta.ini").contains("installationFile="));
    const QStringList o = w.modOrder();
    QCOMPARE(o.indexOf("Quick One") + 1, o.indexOf("SKSE"));
  }

  void aFomodArchiveIsInstalledWithTheChoicesClicked()
  {
    put(m_outside + "/fomod/fomod/ModuleConfig.xml", ModuleConfig);
    put(m_outside + "/fomod/red/textures/red.dds", "red");
    put(m_outside + "/fomod/blue/textures/blue.dds", "blue");
    const QString archive = m_outside + "/Drive Fomod.zip";
    QVERIFY(zipFolder(m_outside + "/fomod", archive));
    auto blueIn = [](QWidget* d) -> QAbstractButton* {
      for (auto* b : d->findChildren<QAbstractButton*>("choice")) {
        if (b->isVisible() && b->text() == "Blue") {
          return b;
        }
      }
      return nullptr;
    };
    bool installed = false;
    whenModal(
        [&](QWidget* d) {
          QPointer<QAbstractButton> blue(blueIn(d));
          QVERIFY(blue);
          QTest::mouseClick(blue, Qt::LeftButton);
          // the step is rebuilt from Rust's answer (new buttons): then Install
          QPointer<QWidget> dialog(d);
          when([=] { return dialog && blueIn(dialog) && blueIn(dialog) != blue && blueIn(dialog)->isChecked(); },
               [=, &installed] {
                 auto* next = dialog->findChild<QPushButton*>("nextBtn");
                 QCOMPARE(next->text(), QString("Install"));
                 installed = true;
                 QTest::mouseClick(next, Qt::LeftButton);
               });
        },
        named("FomodInstallerDialog"));
    auto mime = urls(archive);
    QVERIFY(!dropInstall(mime.get(), onRow(w.modList(), w.mod("Top_separator"))).isEmpty());
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("Drive Fomod").isValid(), 15000);
    QVERIFY(installed);
    QCOMPARE(fileText(w.root + "/mods/Drive Fomod/textures/blue.dds"), QString("blue"));
    QVERIFY(!QFile::exists(w.root + "/mods/Drive Fomod/textures/red.dds"));
    // onto the separator: after its own mods, before its first sub-separator (Nest 1)
    QCOMPARE(w.modOrder().indexOf("Drive Fomod"), w.modOrder().indexOf("Nest 1_separator") - 1);
    QCOMPARE(w.mod("Drive Fomod").parent(), w.mod("Top_separator"));
  }

  void theSameModAgainAsksMergeReplaceOrRename()
  {
    // a second version of "Quick One" with another file: Merge keeps both
    put(m_outside + "/simple2/textures/second.dds", "second");
    const QString archive = m_outside + "/Simple Mod-123-1-1.zip";
    QVERIFY(zipFolder(m_outside + "/simple2", archive));
    whenModal(
        [](QWidget* d) {
          typeName(d->findChild<QComboBox*>("nameCombo"), "Quick One");
          QTest::mouseClick(d->findChild<QPushButton*>("okBtn"), Qt::LeftButton);
        },
        named("SimpleInstallDialog"));
    bool clash = false;
    whenModal(
        [&clash](QWidget* d) {
          clash = true;
          QTest::mouseClick(d->findChild<QPushButton*>("mergeBtn"), Qt::LeftButton);
        },
        named("QueryOverwriteDialog"));
    auto mime = urls(archive);
    QVERIFY(!dropInstall(mime.get(), onRow(w.modList(), w.mod("ENB"))).isEmpty());
    QTRY_VERIFY_WITH_TIMEOUT(QFile::exists(w.root + "/mods/Quick One/textures/second.dds"), 15000);
    QVERIFY(clash);
    QCOMPARE(fileText(w.root + "/mods/Quick One/textures/simple.dds"), QString("simple"));
  }

  void rootFilesAreMovedWhenTheUserSaysMove()
  {
    put(m_outside + "/rooted/skse64_loader.exe", "exe");
    put(m_outside + "/rooted/skse64_1_5_97.dll", "dll");
    put(m_outside + "/rooted/Data/Scripts/x.pex", "pex");
    const QString archive = m_outside + "/Rooted Extender.zip";
    QVERIFY(zipFolder(m_outside + "/rooted", archive));
    QStringList seen;
    // whichever installer asks first: its default, then the Root sorter's question
    whenModal([&seen](QWidget* d) {
      seen << (d->objectName().isEmpty() ? d->windowTitle() : d->objectName());
      if (auto* ok = d->findChild<QPushButton*>("okBtn")) {
        QTest::mouseClick(ok, Qt::LeftButton);
      }
    }, [](QWidget* d) { return d->windowTitle() != "Root files"; });
    bool asked = false;
    whenModal(
        [&asked](QWidget* d) {
          asked = true;
          for (auto* b : d->findChildren<QPushButton*>()) {
            if (plain(b->text()) == "Move") {
              QTest::mouseClick(b, Qt::LeftButton);
            }
          }
        },
        titled("Root files"));
    auto mime = urls(archive);
    QVERIFY(!dropInstall(mime.get(), onRow(w.modList(), w.mod("ENB"))).isEmpty());
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("Rooted Extender").isValid(), 15000);
    QVERIFY2(asked, qPrintable(seen.join(", ")));
    const QString mod = w.root + "/mods/Rooted Extender";
    QVERIFY(QFile::exists(mod + "/Root/skse64_loader.exe"));
    QVERIFY(QFile::exists(mod + "/Root/skse64_1_5_97.dll"));
    QVERIFY(QFile::exists(mod + "/Scripts/x.pex"));
  }

  void aBainArchiveInstallsTheTickedPackages()
  {
    put(m_outside + "/bain/00 Core/textures/core.dds", "core");
    put(m_outside + "/bain/01 Option/textures/option.dds", "option");
    put(m_outside + "/bain/02 Other/textures/other.dds", "other");
    const QString archive = m_outside + "/Bain Pack.zip";
    QVERIFY(zipFolder(m_outside + "/bain", archive));
    whenModal(
        [](QWidget* d) {
          auto* list = d->findChild<QListWidget*>("optionsList");
          QVERIFY(list);
          for (int i = 0; i < list->count(); ++i) {
            auto* item = list->item(i);
            const bool want = item->text() != "02 Other";
            if ((item->checkState() == Qt::Checked) != want) {
              // a click on the item's check box
              const QRect r = list->visualItemRect(item);
              QStyleOptionViewItem o;
              o.initFrom(list);
              o.rect     = r;
              o.features = QStyleOptionViewItem::HasCheckIndicator | QStyleOptionViewItem::HasDisplay;
              o.text     = item->text();
              const QPoint at = list->style()->subElementRect(QStyle::SE_ItemViewItemCheckIndicator, &o, list).center();
              QTest::mouseClick(list->viewport(), Qt::LeftButton, Qt::NoModifier, at);
            }
          }
          for (int i = 0; i < list->count(); ++i) {
            QCOMPARE(list->item(i)->checkState() == Qt::Checked, list->item(i)->text() != "02 Other");
          }
          QTest::mouseClick(d->findChild<QPushButton*>("okBtn"), Qt::LeftButton);
        },
        named("BainComplexInstallerDialog"));
    auto mime = urls(archive);
    QVERIFY(!dropInstall(mime.get(), onRow(w.modList(), w.mod("ENB"))).isEmpty());
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("Bain Pack").isValid(), 15000);
    const QString mod = w.root + "/mods/Bain Pack";
    QVERIFY(QFile::exists(mod + "/textures/core.dds"));
    QVERIFY(QFile::exists(mod + "/textures/option.dds"));
    QVERIFY(!QFile::exists(mod + "/textures/other.dds"));
  }

  void theDownloadsTabHidesAndInstallsByDragAndDoubleClick()
  {
    put(m_outside + "/dl/textures/dl.dds", "dl");
    const QString downloads = w.root + "/downloads";
    QVERIFY(zipFolder(m_outside + "/dl", downloads + "/Downloaded Mod-55-2-0.zip"));
    QVERIFY(QFile::copy(downloads + "/Downloaded Mod-55-2-0.zip", downloads + "/Second Download-56-1-0.zip"));
    w.child<QTabWidget>("tabWidget")->setCurrentWidget(w.child<QWidget>("downloadTab"));
    QTest::mouseClick(w.child<QPushButton>("btnRefreshDownloads"), Qt::LeftButton);
    auto* view = w.child<QTreeView>("downloadView");
    auto row = [&](const QString& file) { return find(view->model(), Qt::ToolTipRole, file); };
    QTRY_VERIFY(row("Downloaded Mod-55-2-0.zip").isValid());

    // Hide (context menu): gone from the list, removed=true in its .meta; "Hidden files" shows it
    clickRow(view, row("Downloaded Mod-55-2-0.zip"));
    whenMenu({"Hide"});
    contextMenu(view, row("Downloaded Mod-55-2-0.zip"));
    QTRY_VERIFY(fileText(downloads + "/Downloaded Mod-55-2-0.zip.meta").contains("removed=true"));
    QTRY_VERIFY(!row("Downloaded Mod-55-2-0.zip").isValid());
    clickBox(w.child<QCheckBox>("showHiddenBox"));
    QTRY_VERIFY(row("Downloaded Mod-55-2-0.zip").isValid());
    clickRow(view, row("Downloaded Mod-55-2-0.zip"));
    whenMenu({"Un-Hide"});
    contextMenu(view, row("Downloaded Mod-55-2-0.zip"));
    QTRY_VERIFY(fileText(downloads + "/Downloaded Mod-55-2-0.zip.meta").contains("removed=false"));

    // dragged onto the mod list, above ENB: installed there
    auto quick = [](const QString& name) {
      whenModal(
          [name](QWidget* d) {
            typeName(d->findChild<QComboBox*>("nameCombo"), name);
            QTest::mouseClick(d->findChild<QPushButton*>("okBtn"), Qt::LeftButton);
          },
          named("SimpleInstallDialog"));
    };
    quick("From Downloads");
    clickRow(view, row("Downloaded Mod-55-2-0.zip"));
    std::unique_ptr<QMimeData> mime(view->model()->mimeData(view->selectionModel()->selectedIndexes()));
    QVERIFY(!dropInstall(mime.get(), aboveRow(w.modList(), w.mod("ENB"))).isEmpty());
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("From Downloads").isValid(), 15000);
    QCOMPARE(w.modOrder().indexOf("From Downloads") + 1, w.modOrder().indexOf("ENB"));
    QTRY_VERIFY(fileText(downloads + "/Downloaded Mod-55-2-0.zip.meta").contains("installed=true"));

    // a double click installs too
    quick("By Double Click");
    QTRY_VERIFY(row("Second Download-56-1-0.zip").isValid());
    doubleClick(view->viewport(), onRow(view, row("Second Download-56-1-0.zip")));
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("By Double Click").isValid(), 15000);
    QCOMPARE(fileText(w.root + "/mods/By Double Click/textures/dl.dds"), QString("dl"));
  }

  void theQuickInstallersManualButtonOpensTheManualInstaller()
  {
    put(m_outside + "/manual/textures/manual.dds", "manual");
    const QString archive = m_outside + "/Manual Mod.zip";
    QVERIFY(zipFolder(m_outside + "/manual", archive));
    whenModal(
        [](QWidget* d) {
          typeName(d->findChild<QComboBox*>("nameCombo"), "By Hand");
          QTest::mouseClick(d->findChild<QPushButton*>("manualBtn"), Qt::LeftButton);
        },
        named("SimpleInstallDialog"));
    bool manual = false;
    whenModal(
        [&manual](QWidget* d) {
          manual = true;
          shoot(d, "manual-installer");
          QTest::mouseClick(d->findChild<QPushButton*>("okButton"), Qt::LeftButton);
        },
        named("InstallDialog"));
    auto mime = urls(archive);
    QVERIFY(!dropInstall(mime.get(), onRow(w.modList(), w.mod("ENB"))).isEmpty());
    QTRY_VERIFY_WITH_TIMEOUT(QFile::exists(w.root + "/mods/By Hand/textures/manual.dds"), 15000);
    QVERIFY(manual);
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("By Hand").isValid(), 15000);  // the list read again
  }

  void anArchiveOfArchivesAsksWhichOne()
  {
    // installer_bundle: the archive holds only archives (with one MO2 takes it without asking;
    // with two it asks, a button per archive)
    put(m_outside + "/inner/textures/inner.dds", "inner");
    QDir().mkpath(m_outside + "/bundle");
    QVERIFY(zipFolder(m_outside + "/inner", m_outside + "/bundle/Inner Mod.zip"));
    QVERIFY(zipFolder(m_outside + "/inner", m_outside + "/bundle/Other Mod.zip"));
    const QString archive = m_outside + "/Bundle.zip";
    QVERIFY(zipFolder(m_outside + "/bundle", archive));
    bool asked = false;
    whenModal(
        [&asked](QWidget* d) {
          asked = true;
          for (auto* b : d->findChildren<QPushButton*>()) {
            if (b->text().contains("Inner Mod.zip")) {
              QTest::mouseClick(b, Qt::LeftButton);
              return;
            }
          }
          QFAIL("no button for the inner archive");
        },
        named("MultiArchiveDialog"));
    // the inner archive then goes through the quick installer
    whenModal([](QWidget* d) { QTest::mouseClick(d->findChild<QPushButton*>("okBtn"), Qt::LeftButton); },
              named("SimpleInstallDialog"));
    auto mime = urls(archive);
    QVERIFY(!dropInstall(mime.get(), onRow(w.modList(), w.mod("ENB"))).isEmpty());
    QTRY_VERIFY_WITH_TIMEOUT(asked, 15000);
    QTRY_VERIFY_WITH_TIMEOUT(QFile::exists(w.root + "/mods/Inner Mod/textures/inner.dds") ||
                                 QFile::exists(w.root + "/mods/Bundle/textures/inner.dds"),
                             15000);
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("Inner Mod").isValid() || w.mod("Bundle").isValid(), 15000);  // the list read again
  }

  void somethingElseDroppedIsRefused()
  {
    put(m_outside + "/notes.txt", "x");
    const QStringList before = w.modOrder();
    auto mime = urls(m_outside + "/notes.txt");
    dropAt(w.modList(), mime.get(), onRow(w.modList(), w.mod("ENB")));
    QTest::qWait(200);
    QCOMPARE(w.modOrder(), before);
    bool said = false;
    for (auto* label : w.child<QStatusBar>("statusBar")->findChildren<QLabel*>()) {
      said = said || label->text().contains("not an archive");
    }
    QVERIFY2(said, "the status bar does not say why");
  }

  // `strata link` (the browser's nxm handler) hands a link over the window's socket: one line
  // in, "ok" or "no <why>" out (strata_ui::handover)
  void anNxmLinkFromTheBrowserIsQueuedForItsGameOnly()
  {
    const QString path = QString::fromStdString(std::string(strata::handover_socket_path()));
    QVERIFY(QFile::exists(path));
    auto ask = [&path](const QString& link) {
      QLocalSocket socket;
      socket.connectToServer(path);
      if (!socket.waitForConnected(2000)) {
        return QString("not connected");
      }
      socket.write((link + "\n").toUtf8());
      // the window answers from this thread's event loop: wait with it running
      QElapsedTimer t;
      t.start();
      while (!socket.canReadLine() && t.elapsed() < 5000) {
        QTest::qWait(20);
      }
      return QString::fromUtf8(socket.readLine()).trimmed();
    };
    const QString other = ask("nxm://skyrim/mods/5/files/77?key=k&expires=1900000000&user_id=9");
    QVERIFY2(other.startsWith("no ") && other.contains("skyrimspecialedition"), qPrintable(other));
    QCOMPARE(ask("nxm://skyrimspecialedition/mods/5/files/77?key=k&expires=1900000000&user_id=9"), QString("ok"));
  }
};

std::unique_ptr<QObject> drive::makeDropsTest()
{
  return std::make_unique<DropsTest>();
}

#include "drops_test.moc"
