#include "downloadactions.h"

#include "downloadqueue.h"
#include "installhost.h"
#include "strata_bridge/bridge_builtins.h"
#include "strata_bridge/bridge_install.h"

#include <QAbstractItemView>
#include <QAction>
#include <QDesktopServices>
#include <QFileDialog>
#include <QFileInfo>
#include <QInputDialog>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLineEdit>
#include <QMenu>
#include <QMessageBox>
#include <QSignalBlocker>
#include <QUrl>

#include <memory>

namespace strata
{

namespace
{

QString qs(const rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

QStringList selectedFiles(QAbstractItemView* view)
{
  QStringList files;
  for (const QModelIndex& index : view->selectionModel()->selectedRows(0)) {
    const QString file = index.data(Qt::ToolTipRole).toString();
    if (!file.isEmpty()) {
      files << file;
    }
  }
  return files;
}

// Installs the files one after the other, each with its dialogs.
void installQueue(QWidget* window, std::shared_ptr<DownloadContext> ctx, QStringList files, QString under)
{
  if (files.isEmpty()) {
    ctx->refresh();
    return;
  }
  const QString file = files.takeFirst();
  const QString archive = ctx->downloads() + "/" + file;
  startInstall(window, ctx->root(), ctx->profile(), archive, under,
               [=](const QString& report, const QString& error) {
                 if (!error.isEmpty()) {
                   ctx->log(QObject::tr("%1: %2").arg(file, error));
                   if (!error.contains("cancelled")) {
                     QMessageBox::warning(window, QObject::tr("Installation failed"), error);
                   }
                 } else {
                   const auto r = QJsonDocument::fromJson(report.toUtf8()).object();
                   const auto installed = r["installed"].toObject();
                   ctx->log(QObject::tr("%1: installed as '%2'").arg(file, installed["name"].toString()));
                   if (!r["deploy"].toString().isEmpty()) {
                     ctx->log(r["deploy"].toString());
                   }
                   if (installed["configure_ini"].toBool() && ctx->configureIni) {
                     ctx->configureIni(installed["name"].toString());
                   }
                 }
                 installQueue(window, ctx, files, under);
               });
}

QString chooseSeparator(QWidget* window, const DownloadContext& ctx, bool* ok)
{
  const auto choices = separator_choices(ctx.root().toStdString(), ctx.profile().toStdString());
  QStringList names;
  for (const auto& s : choices.separators) {
    names << qs(s);
  }
  *ok = false;
  if (names.isEmpty()) {
    QMessageBox::information(window, QObject::tr("Install under separator"), QObject::tr("The mod list has no separators."));
    return {};
  }
  const int current = std::max<qsizetype>(0, names.indexOf(qs(choices.last)));
  return QInputDialog::getItem(window, QObject::tr("Install under separator"), QObject::tr("Separator"), names,
                               int(current), false, ok);
}

}  // namespace

DownloadQueue* attachDownloadActions(QAbstractItemView* view, QWidget* window, DownloadContext context)
{
  auto ctx = std::make_shared<DownloadContext>(std::move(context));
  auto* queue = new DownloadQueue(view, ctx);
  auto askLink = [=] {
    bool ok = false;
    const QString link = QInputDialog::getText(window, QObject::tr("Download"), QObject::tr("nxm:// link"),
                                               QLineEdit::Normal, QString(), &ok)
                             .trimmed();
    if (ok && !link.isEmpty()) {
      queue->addLink(link);
    }
  };

  auto act = [=](const QStringList& files, const char* action) {
    for (const QString& file : files) {
      try {
        const QString line = qs(download_action(ctx->root().toStdString(), file.toStdString(), action));
        if (!line.isEmpty()) {
          ctx->log(line);
        }
      } catch (const rust::Error& e) {
        ctx->log(QString::fromUtf8(e.what()));
      }
    }
    ctx->refresh();
  };

  QObject::connect(view, &QAbstractItemView::doubleClicked, window, [=](const QModelIndex& index) {
    const QString file = index.siblingAtColumn(0).data(Qt::ToolTipRole).toString();
    if (!file.isEmpty()) {
      installQueue(window, ctx, {file}, QString());
    }
  });


  view->setContextMenuPolicy(Qt::CustomContextMenu);
  QObject::connect(view, &QWidget::customContextMenuRequested, window, [=](const QPoint& pos) {
    const QStringList files = selectedFiles(view);
    if (files.isEmpty()) {
      QMenu menu;
      menu.addAction(QObject::tr("Download nxm:// link..."), askLink);
      menu.exec(view->viewport()->mapToGlobal(pos));
      return;
    }
    const QString first = ctx->downloads() + "/" + files.first();
    QMenu menu;
    QStringList paused;
    for (const QString& f : files) {
      if (f.endsWith(".unfinished") && f != queue->running() + ".unfinished") {
        paused << f;
      }
    }
    if (!queue->running().isEmpty() && files.contains(queue->running() + ".unfinished")) {
      menu.addAction(QObject::tr("Pause"), [=] { queue->pause(queue->running()); });
      menu.addSeparator();
    }
    if (!paused.isEmpty()) {
      menu.addAction(QObject::tr("Resume"), [=] {
        for (const QString& f : paused) {
          queue->resume(f);
        }
      });
      menu.addSeparator();
    }
    menu.addAction(files.size() == 1 ? QObject::tr("Install") : QObject::tr("Install %1 selected").arg(files.size()),
                   [=] { installQueue(window, ctx, files, QString()); });
    menu.addAction(QObject::tr("Install under separator..."), [=] {
      bool ok = false;
      const QString under = chooseSeparator(window, *ctx, &ok);
      if (ok) {
        installQueue(window, ctx, files, under);
      }
    });
    menu.addSeparator();
    menu.addAction(QObject::tr("Query Info"), [=] { act(files, "query"); });
    menu.addAction(QObject::tr("Open File"), [=] { QDesktopServices::openUrl(QUrl::fromLocalFile(first)); });
    menu.addAction(QObject::tr("Open Meta File"), [=] {
      if (QFileInfo::exists(first + ".meta")) {
        QDesktopServices::openUrl(QUrl::fromLocalFile(first + ".meta"));
      }
    });
    menu.addAction(QObject::tr("Show in Folder"),
                   [=] { QDesktopServices::openUrl(QUrl::fromLocalFile(QFileInfo(first).absolutePath())); });
    menu.addSeparator();
    menu.addAction(QObject::tr("Mark Installed"), [=] { act(files, "installed"); });
    menu.addAction(QObject::tr("Mark Uninstalled"), [=] { act(files, "uninstalled"); });
    menu.addAction(QObject::tr("Hide"), [=] { act(files, "hide"); });
    menu.addAction(QObject::tr("Un-Hide"), [=] { act(files, "unhide"); });
    menu.addSeparator();
    menu.addAction(QObject::tr("Delete..."), [=] {
      const auto answer = QMessageBox::question(
          window, QObject::tr("Delete Files?"),
          QObject::tr("This will move the selected download(s) and their meta files to the trash."),
          QMessageBox::Yes | QMessageBox::No);
      if (answer == QMessageBox::Yes) {
        act(files, "delete");
      }
    });
    menu.addSeparator();
    menu.addAction(QObject::tr("Download nxm:// link..."), askLink);
    menu.exec(view->viewport()->mapToGlobal(pos));
  });
  return queue;
}

InstallActions attachInstallActions(QAction* installMod, QAction* autoEnable, QWidget* window,
                                    DownloadContext context)
{
  auto ctx = std::make_shared<DownloadContext>(std::move(context));
  installMod->setEnabled(true);
  installMod->setToolTip(installMod->text().remove('&'));
  QObject::connect(installMod, &QAction::triggered, window, [=] {
    if (ctx->root().isEmpty()) {
      return;
    }
    const QString file = QFileDialog::getOpenFileName(
        window, QObject::tr("Choose Mod"), ctx->downloads(),
        QObject::tr("Mod Archive") + " (*.7z *.zip *.rar *.fomod *.001)");
    if (!file.isEmpty()) {
      startInstall(window, ctx->root(), ctx->profile(), file, QString(),
                   [=](const QString& report, const QString& error) {
                     if (!error.isEmpty()) {
                       ctx->log(error);
                       if (!error.contains("cancelled")) {
                         QMessageBox::warning(window, QObject::tr("Installation failed"), error);
                       }
                     } else {
                       const auto r = QJsonDocument::fromJson(report.toUtf8()).object();
                       const auto installed = r["installed"].toObject();
                       ctx->log(QObject::tr("installed as '%1'").arg(installed["name"].toString()));
                       if (installed["configure_ini"].toBool() && ctx->configureIni) {
                         ctx->configureIni(installed["name"].toString());
                       }
                     }
                     ctx->refresh();
                   });
    }
  });

  autoEnable->setEnabled(true);
  autoEnable->setCheckable(true);
  autoEnable->setToolTip(autoEnable->text().remove('&'));
  auto sync = [=] {
    QSignalBlocker block(autoEnable);
    autoEnable->setChecked(ctx->root().isEmpty() || auto_enable(ctx->root().toStdString()));
  };
  sync();
  QObject::connect(autoEnable, &QAction::toggled, window, [=](bool on) {
    try {
      set_auto_enable(ctx->root().toStdString(), on);
    } catch (const rust::Error& e) {
      ctx->log(QString::fromUtf8(e.what()));
    }
  });
  return {sync};
}

}  // namespace strata
