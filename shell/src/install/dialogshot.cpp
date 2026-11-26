// strata-dialog-shot <kind> <request.json> <out.png> [answer.json]
// strata-dialog-shot install <instance> <profile> <archive> <out prefix>
// strata-dialog-shot download <instance> <nxm link> <out prefix>
//
// The third form drives the Downloads tab's queue (DownloadQueue) on a list of the
// downloads folder: the link is queued, paused at its first progress report, resumed, and
// <out prefix>-paused.png / -done.png are saved; prints the log lines.
//
// The second form runs a whole install through the window's code path (worker thread,
// InstallDialogs, the bridge) and presses each dialog's OK / Next / Install / Move / Merge
// button, saving <out prefix>-<n>.png of every dialog; prints the report.
//
// Shows one installer dialog (InstallDialogs::show) with a request read from a file, saves a
// picture of it after it is laid out, closes it, and writes the dialog's answer to
// answer.json (or stdout). For the screenshot checks (tools/shell-test-install.sh); run with
// QT_QPA_PLATFORM=offscreen. The FOMOD dialog's first answer is the one written.

#include "install/downloadqueue.h"
#include "install/installhost.h"

#include <QApplication>
#include <QDir>
#include <QStandardItemModel>
#include <QHeaderView>
#include <QTreeView>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QPushButton>
#include <QTimer>
#include <QWidget>

#include <cstdio>

namespace
{

int installRun(QApplication& app, char** argv)
{
  QWidget parent;
  parent.resize(1000, 700);
  const QString prefix = QString::fromLocal8Bit(argv[5]);
  int shots = 0, code = 1;
  QTimer timer;
  QObject::connect(&timer, &QTimer::timeout, [&] {
    for (QWidget* w : QApplication::topLevelWidgets()) {
      if (w == &parent || !w->isVisible() || !w->inherits("QDialog") || w->inherits("QProgressDialog")) {
        continue;
      }
      w->grab().save(QString("%1-%2.png").arg(prefix).arg(++shots));
      for (const char* text : {"Install", "Next", "OK", "Ok", "Move", "Merge"}) {
        for (auto* b : w->findChildren<QPushButton*>()) {
          if (b->isVisible() && b->isEnabled() && b->text().remove('&') == QLatin1String(text)) {
            b->click();
            return;
          }
        }
      }
      return;
    }
  });
  timer.start(500);
  strata::startInstall(&parent, QString::fromLocal8Bit(argv[2]), QString::fromLocal8Bit(argv[3]),
                       QString::fromLocal8Bit(argv[4]), QString(),
                       [&](const QString& report, const QString& error) {
                         std::printf("%s\n", qUtf8Printable(error.isEmpty() ? report : "error: " + error));
                         code = error.isEmpty() ? 0 : 1;
                         app.quit();
                       });
  app.exec();
  return code;
}

// A list like the Downloads tab: Name (tooltip = file) and Status, reloaded on refresh.
int downloadRun(QApplication& app, char** argv)
{
  const QString root = QString::fromLocal8Bit(argv[2]);
  const QString prefix = QString::fromLocal8Bit(argv[4]);
  QStandardItemModel model;
  model.setHorizontalHeaderLabels({"Name", "Status"});
  QTreeView view;
  view.setModel(&model);
  view.resize(700, 160);
  view.header()->resizeSection(0, 420);
  view.show();
  auto reload = [&] {
    model.removeRows(0, model.rowCount());
    for (const QString& f : QDir(root + "/downloads").entryList({"*.7z", "*.7z.unfinished"}, QDir::Files)) {
      auto* name = new QStandardItem(f);
      name->setToolTip(f);
      model.appendRow({name, new QStandardItem(f.endsWith(".unfinished") ? "Paused" : "Downloaded")});
    }
  };
  strata::DownloadContext ctx;
  ctx.root = [=] { return root; };
  ctx.profile = [] { return QString("Default"); };
  ctx.downloads = [=] { return root + "/downloads"; };
  int stage = 0, code = 1;
  strata::DownloadQueue* queue = nullptr;
  ctx.log = [&](const QString& line) {
    std::printf("%s\n", qUtf8Printable(line));
    std::fflush(stdout);
    if (line.startsWith("paused")) {
      reload();
      view.grab().save(prefix + "-paused.png");
      stage = 2;
      queue->resume(line.mid(7));
    } else if (line.startsWith("downloaded")) {
      reload();
      view.grab().save(prefix + "-done.png");
      code = 0;
      app.quit();
    } else if (line.startsWith("download failed")) {
      app.quit();
    }
  };
  ctx.refresh = reload;
  reload();
  queue = new strata::DownloadQueue(&view, std::make_shared<strata::DownloadContext>(ctx));
  QTimer poll;
  QObject::connect(&poll, &QTimer::timeout, [&] {
    // pause as soon as the row shows progress
    for (int r = 0; stage == 0 && r < model.rowCount(); ++r) {
      if (model.item(r, 1)->text().startsWith("Downloading")) {
        view.grab().save(prefix + "-running.png");
        queue->pause(queue->running());
        stage = 1;
      }
    }
  });
  poll.start(5);
  queue->addLink(QString::fromLocal8Bit(argv[3]));
  QTimer::singleShot(60000, &app, [&] { app.quit(); });
  app.exec();
  return code;
}

}  // namespace

int main(int argc, char** argv)
{
  QApplication app(argc, argv);
  if (argc == 6 && QString::fromLocal8Bit(argv[1]) == "install") {
    return installRun(app, argv);
  }
  if (argc == 5 && QString::fromLocal8Bit(argv[1]) == "download") {
    return downloadRun(app, argv);
  }
  if (argc < 4) {
    std::fprintf(stderr, "usage: strata-dialog-shot <kind> <request.json> <out.png> [answer.json]\n");
    return 2;
  }
  const QString kind = QString::fromLocal8Bit(argv[1]);
  QFile file(QString::fromLocal8Bit(argv[2]));
  if (!file.open(QIODevice::ReadOnly)) {
    std::fprintf(stderr, "cannot read %s\n", argv[2]);
    return 2;
  }
  const auto request = QJsonDocument::fromJson(file.readAll()).object();
  const QString out = QString::fromLocal8Bit(argv[3]);
  // as the window applies a sheet ("file:///" + absolute path)
  if (const QByteArray sheet = qgetenv("STRATA_STYLE_FILE"); !sheet.isEmpty()) {
    app.setStyleSheet("file:///" + QFileInfo(QString::fromLocal8Bit(sheet)).absoluteFilePath());
  }

  QWidget parent;
  parent.resize(1000, 700);
  strata::InstallDialogs host(&parent);
  bool saved = false;
  // grab whatever top-level dialog is up, then close it (Escape = reject / cancel)
  QTimer timer;
  QObject::connect(&timer, &QTimer::timeout, [&] {
    for (QWidget* w : QApplication::topLevelWidgets()) {
      if (w != &parent && w->isVisible() && w->inherits("QDialog")) {
        if (!saved) {
          saved = w->grab().save(out);
        }
        if (w->inherits("strata::FomodDialog")) {
          w->close();  // its closeEvent answers "cancel"
        } else {
          QMetaObject::invokeMethod(w, "reject");
        }
        return;
      }
    }
  });
  timer.start(400);
  const auto answer = host.show(kind, request);
  const QByteArray json = QJsonDocument(answer).toJson(QJsonDocument::Compact);
  if (argc > 4) {
    QFile a(QString::fromLocal8Bit(argv[4]));
    if (a.open(QIODevice::WriteOnly)) {
      a.write(json);
    }
  } else {
    std::printf("%s\n", json.constData());
  }
  return saved ? 0 : 1;
}
