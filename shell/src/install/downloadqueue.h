#pragma once

// The Downloads tab's queue (MO2's DownloadManager, one download at a time): nxm:// links
// and resumed paused downloads run on a worker thread through the bridge
// (strata_ui::nexus); the row's Status column shows the percentage, Pause stops after the
// current chunk (the file stays <archive>.unfinished, listed as Paused).

#include "downloadactions.h"
#include "installhost_api.h"

#include <QObject>
#include <QStringList>

#include <atomic>
#include <memory>

class QAbstractItemView;

namespace strata
{

class DownloadQueue : public QObject, public DownloadHost
{
public:
  DownloadQueue(QAbstractItemView* view, std::shared_ptr<DownloadContext> ctx);

  // Queues the file of an nxm:// link.
  void addLink(const QString& link);
  // Queues a paused download (its row's file, with or without ".unfinished").
  void resume(const QString& file);
  // Pauses the running download if it is `file`.
  void pause(const QString& file);
  // The archive downloading now ("" when idle).
  QString running() const { return m_running; }

  bool progress(rust::Str file, std::uint64_t done, std::uint64_t total) override;

private:
  struct Job
  {
    bool link;
    QString what;
  };

  void startNext();
  void showStatus(const QString& file, const QString& text);

  QAbstractItemView* m_view;
  std::shared_ptr<DownloadContext> m_ctx;
  QList<Job> m_jobs;
  bool m_busy = false;
  QString m_running;
  std::atomic_bool m_pause{false};
  // worker thread only: what progress() reported last
  QString m_reported;
  int m_lastPercent = -1;
};

}  // namespace strata
