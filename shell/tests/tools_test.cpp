// The M6 dialogs from their menu entries (Tool Plugins, Notifications), each changed and
// accepted, the file it writes checked; the executables dialog from Tools; the mod info
// dialog from "Information..."; the installer from Install Mod... (its file dialog).

#include "tests.h"

#include <QColorDialog>
#include <QDir>
#include <QFileDialog>
#include <QInputDialog>
#include <QLabel>
#include <QRegularExpression>
#include <QListWidget>
#include <QMessageBox>
#include <QProcess>
#include <QTabWidget>
#include <QTest>
#include <QTextEdit>
#include <QToolBar>
#include <QTreeWidget>

using namespace drive;

namespace
{

// Opens the toolbar's Tool Plugins menu and picks `entry`.
void toolMenu(QMainWindow* window, const QString& entry)
{
  // a popup still open from the test before would only be closed by the click, the menu never
  // opening and the armed handler leaking into the next test
  QTRY_VERIFY(QApplication::activePopupWidget() == nullptr);
  whenMenu({entry});
  for (auto* bar : window->findChildren<QToolBar*>()) {
    if (QWidget* b = bar->widgetForAction(window->findChild<QAction*>("actionTool"))) {
      QTest::mouseClick(b, Qt::LeftButton);
    }
  }
}

QPushButton* buttonText(QWidget* parent, const QString& text)
{
  for (auto* b : parent->findChildren<QPushButton*>()) {
    if (plain(b->text()) == text && b->isVisible()) {
      return b;
    }
  }
  return nullptr;
}

std::function<bool(QWidget*)> kindOf(const char* cls)
{
  return [cls](QWidget* w) { return w->inherits(cls); };
}

// Answers a file dialog with `path` (Qt's own dialog: the offscreen platform has no native one).
void answerFileDialog(const QString& path)
{
  whenModalOf<QFileDialog>([path](QFileDialog* d) {
    // typed into the name box: QFileDialog::accept reads it (its folder may still be loading)
    if (auto* name = d->findChild<QLineEdit*>("fileNameEdit")) {
      name->setText(path);
    } else {
      d->selectFile(path);
    }
    QMetaObject::invokeMethod(d, "accept");  // QDialog::accept is protected in QFileDialog
  });
}

}  // namespace

class ToolsTest : public QObject
{
  Q_OBJECT

  WindowFixture w;

private slots:
  void initTestCase()
  {
    const QString root = makeInstance("tools");
    // an executable started from a mod's Root folder: the Problems dialog has a fix for it
    QString ini = fileText(root + "/ModOrganizer.ini");
    const QString binary = ini.section("1\\binary=", 1).section('\r', 0, 0);
    ini.replace("1\\binary=" + binary, "1\\binary=" + root + "/mods/SKSE/Root/skse64_loader.exe");
    QFile f(root + "/ModOrganizer.ini");
    QVERIFY(f.open(QIODevice::WriteOnly | QIODevice::Truncate));
    f.write(ini.toUtf8());
    f.close();
    QVERIFY(w.open(root));
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("SKSE").isValid(), 15000);
  }

  void cleanupTestCase() { w.close(); }

  void theIniEditorSavesAnEdit()
  {
    const QString file = w.profileFile("skyrim.ini");
    QVERIFY(QFile::exists(file));
    whenModal(
        [](QWidget* d) {
          shoot(d, "ini-editor");
          auto* tabs = d->findChild<QTabWidget*>("editorTabs");
          QVERIFY(tabs && tabs->count() > 0);
          for (int i = 0; i < tabs->count(); ++i) {
            if (tabs->tabText(i).compare("skyrim.ini", Qt::CaseInsensitive) == 0) {
              tabs->setCurrentIndex(i);
            }
          }
          auto* editor = tabs->currentWidget()->findChild<QTextEdit*>("editorView");
          editor->setFocus();
          editor->moveCursor(QTextCursor::End);
          QTest::keyClick(editor, Qt::Key_Return);
          QTest::keyClicks(editor, "sDriveTest=1");
          QTest::mouseClick(buttonText(tabs->currentWidget(), "Save"), Qt::LeftButton);
          d->close();
        },
        kindOf("strata::modlist::IniEditorDialog"));
    toolMenu(w.window.get(), "INI Editor");
    QTRY_VERIFY(fileText(file).contains("sDriveTest=1"));
    QVERIFY(fileText(file).contains("\r\n"));  // TextViewer saves with CRLF
  }

  void theProblemsDialogFixesAnExecutableInARootFolder()
  {
    QVERIFY(w.ini().contains("mods/SKSE/Root/skse64_loader.exe"));
    whenModal(
        [](QWidget* d) {
          auto* tree = d->findChild<QTreeWidget*>("problemsWidget");
          QPointer<QWidget> dialog(d);
          // the checks run on a thread: wait for the row and its Fix button
          when([=] { return dialog && buttonText(dialog, "Fix"); }, [=] {
            shoot(dialog, "problems");
            QTest::mouseClick(buttonText(dialog, "Fix"), Qt::LeftButton);
            when([=] { return dialog && !buttonText(dialog, "Fix") && tree->topLevelItemCount() >= 0; },
                 [=] { QTest::mouseClick(dialog->findChild<QPushButton*>("closeButton"), Qt::LeftButton); });
          });
        },
        kindOf("strata::modlist::ProblemsDialog"));
    w.window->findChild<QAction*>("actionNotifications")->trigger();
    QTRY_VERIFY_WITH_TIMEOUT(!w.ini().contains("mods/SKSE/Root/skse64_loader.exe"), 15000);
    QVERIFY(w.ini().contains("Game Root/skse64_loader.exe"));
  }




  void theExecutablesDialogFromToolsRenamesOne()
  {
    whenModal(
        [](QWidget* d) {
          auto* title = d->findChild<QLineEdit*>("title");
          title->selectAll();
          QTest::keyClicks(title, "Loader");
          QTest::mouseClick(d->findChild<QDialogButtonBox*>("buttons")->button(QDialogButtonBox::Ok), Qt::LeftButton);
        },
        named("EditExecutablesDialog"));
    w.window->findChild<QAction*>("actionModify_Executables")->trigger();
    QTRY_VERIFY(w.ini().contains("1\\title=Loader"));
    QTRY_VERIFY(w.child<QComboBox>("executablesListBox")->findText("Loader") > 0);
  }

  void theModInfoDialogFromInformationSavesACategoryNote()
  {
    whenModal(
        [](QWidget* d) {
          d->findChild<QTabWidget*>("tabWidget")->setCurrentWidget(d->findChild<QWidget*>("tabNotes"));
          auto* comments = d->findChild<QLineEdit*>("comments");
          comments->setFocus();
          QTest::keyClicks(comments, "via Information");
          QTest::mouseClick(d->findChild<QPushButton*>("close"), Qt::LeftButton);
        },
        named("ModInfoDialog"));
    clickRow(w.modList(), w.mod("Deep"));
    whenMenu({"Information..."});
    contextMenu(w.modList(), w.mod("Deep"));
    QTRY_VERIFY(fileText(w.root + "/mods/Deep/meta.ini").contains("comments=via Information"));
  }

  void installModPicksAnArchiveAndInstallsIt()
  {
    // an archive in the downloads folder, picked in Install Mod...'s file dialog
    const QString src = stage() + "/tools-archive";
    QDir(src).removeRecursively();
    QDir().mkpath(src + "/textures");
    QFile tex(src + "/textures/picked.dds");
    QVERIFY(tex.open(QIODevice::WriteOnly));
    tex.write("picked");
    tex.close();
    const QString archive = w.root + "/downloads/Picked Mod-77-1-0.zip";
    QProcess zip;
    zip.setWorkingDirectory(src);
    zip.start("zip", {"-qr", archive, "."});
    QVERIFY(zip.waitForFinished(30000) && zip.exitCode() == 0);
    answerFileDialog(archive);
    whenModal(
        [](QWidget* d) {
          typeName(d->findChild<QComboBox*>("nameCombo"), "Picked");
          QTest::mouseClick(d->findChild<QPushButton*>("okBtn"), Qt::LeftButton);
        },
        named("SimpleInstallDialog"));
    w.window->findChild<QAction*>("actionInstallMod")->trigger();
    QTRY_VERIFY_WITH_TIMEOUT(QFile::exists(w.root + "/mods/Picked/textures/picked.dds"), 15000);
    QTRY_VERIFY(w.mod("Picked").isValid());
  }

  // crashlogtools: the viewer lists a prefix's crash logs newest first, labels the AE one
  // (and leaves the 1.5.97 one), Delete trashes a log, Enter opens one
  void theCrashLogViewerLabelsListsDeletesAndOpens()
  {
    const QString prefix = stage() + "/tools-prefix";
    QDir(prefix).removeRecursively();
    const QString logs = prefix + "/pfx/drive_c/users/steamuser/Documents/My Games/Skyrim Special Edition/SKSE";
    QVERIFY(QDir().mkpath(logs));
    QString ini = fileText(w.root + "/ModOrganizer.ini");
    if (!ini.contains("[fluorine]")) {
      QFile f(w.root + "/ModOrganizer.ini");
      QVERIFY(f.open(QIODevice::Append));
      f.write("[fluorine]\r\nprefix_path=" + prefix.toUtf8() + "\r\n");
    }
    const QString real = fileText(repo() + "/crates/strata-ui/tests/data/crash-2026-11-25-19-39-28.log");
    QVERIFY(real.contains("Skyrim SSE v1.5.97"));
    auto write = [](const QString& path, const QString& text) {
      QFile f(path);
      QVERIFY(f.open(QIODevice::WriteOnly | QIODevice::Truncate));
      f.write(text.toUtf8());
    };
    const QString old = logs + "/crash-2026-11-25-19-39-28.log";
    const QString ae  = logs + "/crash-2026-11-26-10-00-00.log";
    write(old, real);
    write(ae, QString(real).replace("Skyrim SSE v1.5.97", "Skyrim SSE v1.6.1170"));
    write(logs + "/skse64.log", "not a crash log");
    const QString db = stage() + "/data/strata/crashlogs";
    QVERIFY(QDir().mkpath(db));
    write(db + "/skyrimae.rename", "1\n517081 BSDriveTest::Thing_*\n");
    openedUrls().clear();

    whenModal(
        [old, ae](QWidget* d) {
          shoot(d, "crash-log-viewer");
          QVERIFY(d->windowTitle().startsWith("Crash Log Viewer - "));
          auto* list = d->findChild<QListWidget*>("crashLogs");
          QCOMPARE(list->count(), 2);
          QCOMPARE(list->item(0)->text(), QString("crash-2026-11-26-10-00-00.log"));  // newest first
          QVERIFY(d->findChild<QLabel*>("crashLogStatus")->text().contains("labelled 1"));
          // Delete the old one (context menu action)
          list->clearSelection();
          list->item(1)->setSelected(true);
          for (auto* a : list->actions()) {
            if (plain(a->text()) == "Delete") {
              a->trigger();
            }
          }
          QCOMPARE(list->count(), 1);
          // Enter on the other opens it
          list->setCurrentRow(0);
          list->item(0)->setSelected(true);
          list->setFocus();
          QTest::keyClick(list, Qt::Key_Return);
          clickButton(d, QDialogButtonBox::Close);
        },
        kindOf("strata::modlist::CrashLogViewer"));
    toolMenu(w.window.get(), "Crash Log Viewer");
    QTRY_VERIFY(fileText(ae).contains(QRegularExpression("-> 517081\\+0x40 +BSDriveTest::Thing\tadd \\[rax\\], al")));
    QTRY_VERIFY(!QFile::exists(old));
    QVERIFY(QFile::exists(stage() + "/data/Trash/files/crash-2026-11-25-19-39-28.log"));
    QTRY_VERIFY(openedUrls().contains(QUrl::fromLocalFile(ae)));
  }

};

std::unique_ptr<QObject> drive::makeToolsTest()
{
  return std::make_unique<ToolsTest>();
}

#include "tools_test.moc"
