// strata-shell-test: the real window and dialogs driven offscreen with Qt Test, on
// instances made by tools/gen-instance. Every test acts as a user (clicks, typed keys,
// drops, context menus, dialogs answered) and checks the files MO2 would read.
//
// Run by tools/shell-drive.sh (tools/test.sh runs it). The program goes through Strata's own
// entry point, so `strata-shell-test vfsd ...` works as the overlay daemon a deploy starts
// (std::env::current_exe). Qt Test arguments come from STRATA_TEST_ARGS ("-v2", a test
// function name...), since the command line is Strata's.

#include "driver.h"
#include "loglist.h"
#include "mainwindow.h"
#include "modlist.h"
#include "style.h"

#include "tests.h"

#include <QDesktopServices>
#include <QDir>
#include <QProcess>
#include <QTemporaryDir>
#include <QTest>

extern "C" int strata_run(int argc, char** argv,
                          int (*shell)(int, char**, const char*, const char*));

namespace drive
{

QString repo()
{
  return qEnvironmentVariable("STRATA_REPO");
}

QString stage()
{
  return repo() + "/_stage/shell-drive";
}

QString makeInstance(const QString& name)
{
  const QString dir = stage() + "/" + name;
  QDir(dir).removeRecursively();
  QProcess p;
  p.start(repo() + "/tools/gen-instance", {dir});
  if (!p.waitForFinished(60000) || p.exitCode() != 0) {
    qFatal("tools/gen-instance %s failed: %s", qUtf8Printable(dir), p.readAllStandardError().constData());
  }
  return dir;
}

bool copyTree(const QString& from, const QString& to)
{
  QDir().mkpath(to);
  const QDir source(from);
  for (const QFileInfo& e : source.entryInfoList(QDir::AllEntries | QDir::NoDotAndDotDot | QDir::Hidden)) {
    const QString target = to + "/" + e.fileName();
    if (e.isDir() ? !copyTree(e.filePath(), target) : !QFile::copy(e.filePath(), target)) {
      return false;
    }
  }
  return true;
}

QList<QUrl>& openedUrls()
{
  static QList<QUrl> urls;
  return urls;
}

}  // namespace drive

// Takes the URLs QDesktopServices would hand to the desktop.
class UrlCatcher : public QObject
{
  Q_OBJECT

public slots:
  void open(const QUrl& url) { drive::openedUrls() << url; }
};

namespace
{

int testEntry(int argc, char** argv, const char*, const char*)
{
  if (drive::repo().isEmpty()) {
    fprintf(stderr, "set STRATA_REPO to the repository (tools/shell-drive.sh does)\n");
    return 2;
  }
  QDir().mkpath(drive::stage());
  QDir(drive::stage() + "/data").removeRecursively();  // the trash of the last run
  QDir(drive::stage() + "/config").removeRecursively();
  // never the user's config, last instance or trash
  qputenv("XDG_CONFIG_HOME", (drive::stage() + "/config").toUtf8());
  qputenv("XDG_DATA_HOME", (drive::stage() + "/data").toUtf8());
  // nor the user's link sockets (a browser link must not reach a test window), keyring or Nexus
  // (a socket path holds at most 107 bytes: a deep checkout, such as a worktree, gets a short
  // folder under /tmp instead)
  QString run = drive::stage() + "/run";
  static QTemporaryDir shortRun(QDir::tempPath() + "/strata-drive-XXXXXX");
  if ((run + "/strata/window-4194304.sock").toUtf8().size() > 107 && shortRun.isValid()) {
    run = shortRun.path();
  }
  QDir().mkpath(run);
  QFile::setPermissions(run, QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner);
  qputenv("XDG_RUNTIME_DIR", run.toUtf8());
  qputenv("XDG_CACHE_HOME", (drive::stage() + "/cache").toUtf8());
  qputenv("STRATA_NEXUS_API", "http://127.0.0.1:9");
  qputenv("STRATA_SECRET_TOOL", "/bin/false");
  // a test stuck in a dialog fails after a minute (Qt Test waits five by default)
  if (qEnvironmentVariableIsEmpty("QTEST_FUNCTION_TIMEOUT")) {
    qputenv("QTEST_FUNCTION_TIMEOUT", "60000");
  }
  QApplication app(argc, argv);
  QApplication::setApplicationName("strata");
  UrlCatcher catcher;
  for (const char* scheme : {"file", "http", "https"}) {
    QDesktopServices::setUrlHandler(scheme, &catcher, "open");
  }

  QStringList args{QString::fromLocal8Bit(argv[0])};
  args << qEnvironmentVariable("STRATA_TEST_ARGS").split(' ', Qt::SkipEmptyParts);
  // test function names: each class runs the ones it has (none of them: the class is skipped)
  QStringList names, options;
  for (const QString& a : args.mid(1)) {
    (a.startsWith('-') ? options : names) << a;
  }
  int failed = 0;
  for (const auto& make : drive::testClasses()) {
    std::unique_ptr<QObject> test = make();
    QStringList mine;
    for (const QString& n : names) {
      if (test->metaObject()->indexOfMethod(qPrintable(n + "()")) >= 0) {
        mine << n;
      }
    }
    if (!names.isEmpty() && mine.isEmpty()) {
      continue;
    }
    failed += QTest::qExec(test.get(), QStringList{args[0]} + options + mine);
  }
  return failed == 0 ? 0 : 1;
}

}  // namespace

int main(int argc, char** argv)
{
  return strata_run(argc, argv, testEntry);
}

#include "drive.moc"
