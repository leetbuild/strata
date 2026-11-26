#include "downloadqueue.h"

#include "strata_bridge/bridge_install.h"

#include <QAbstractItemView>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocale>
#include <QThread>

namespace strata
{

namespace
{
const QString Unfinished = QStringLiteral(".unfinished");

QString baseName(const QString& file)
{
  return file.endsWith(Unfinished) ? file.left(file.size() - Unfinished.size()) : file;
}
}  // namespace

DownloadQueue::DownloadQueue(QAbstractItemView* view, std::shared_ptr<DownloadContext> ctx)
    : QObject(view), m_view(view), m_ctx(std::move(ctx))
{}

void DownloadQueue::addLink(const QString& link)
{
  m_jobs.append({true, link});
  m_ctx->log(tr("queued %1").arg(link));
  startNext();
}

void DownloadQueue::resume(const QString& file)
{
  m_jobs.append({false, baseName(file)});
  startNext();
}

void DownloadQueue::pause(const QString& file)
{
  if (!m_running.isEmpty() && baseName(file) == m_running) {
    m_pause = true;
  }
}

bool DownloadQueue::progress(rust::Str file, std::uint64_t done, std::uint64_t total)
{
  const QString name = QString::fromUtf8(file.data(), qsizetype(file.size()));
  const int percent = total > 0 ? int(done * 100 / total) : -1;
  if (percent != m_lastPercent || name != m_reported) {
    const bool first = name != m_reported;
    m_reported = name;
    m_lastPercent = percent;
    QMetaObject::invokeMethod(this, [this, name, percent, first, done] {
      if (first) {
        m_running = name;
        m_ctx->refresh();  // the new <archive>.unfinished row
      }
      showStatus(name, percent >= 0 ? tr("Downloading %1%").arg(percent)
                                    : tr("Downloading %1").arg(QLocale().formattedDataSize(qint64(done))));
    });
  }
  return !m_pause;
}

// The row whose file (Name's tooltip) is `file` or `file.unfinished` gets `text` as Status.
void DownloadQueue::showStatus(const QString& file, const QString& text)
{
  auto* model = m_view->model();
  for (int row = 0; row < model->rowCount(); ++row) {
    const QString f = model->index(row, 0).data(Qt::ToolTipRole).toString();
    if (f == file || f == file + Unfinished) {
      model->setData(model->index(row, 1), text);
    }
  }
}

void DownloadQueue::startNext()
{
  if (m_busy || m_jobs.isEmpty()) {
    return;
  }
  const Job job = m_jobs.takeFirst();
  m_busy = true;
  m_pause = false;
  m_lastPercent = -1;
  m_reported.clear();
  m_running = job.link ? QString() : job.what;
  const std::string root = m_ctx->root().toStdString();
  const std::string what = job.what.toStdString();
  auto result = std::make_shared<std::pair<QString, QString>>();
  QThread* worker = QThread::create([this, job, root, what, result] {
    try {
      auto r = job.link ? nexus_download(root, what, *this) : resume_download(root, what, *this);
      result->first = QString::fromUtf8(r.data(), qsizetype(r.size()));
    } catch (const rust::Error& e) {
      result->second = QString::fromUtf8(e.what());
    }
  });
  connect(worker, &QThread::finished, this, [this, worker, result, job] {
    worker->deleteLater();
    if (!result->second.isEmpty()) {
      m_ctx->log(tr("download failed (%1): %2").arg(job.what, result->second));
    } else {
      const auto r = QJsonDocument::fromJson(result->first.toUtf8()).object();
      m_ctx->log(r["done"].toBool() ? tr("downloaded %1").arg(r["file"].toString())
                                    : tr("paused %1").arg(r["file"].toString()));
    }
    m_busy = false;
    m_running.clear();
    m_ctx->refresh();
    startNext();
  });
  worker->start();
}

}  // namespace strata
