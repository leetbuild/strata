#pragma once

// The installer dialogs (install area). The install itself runs in Rust on a worker
// thread (strata::install_archive); each question arrives here as one JSON request
// (crates/strata-ui/src/install.rs lists them), is shown on the UI thread and answered.

#include "install/installhost_api.h"

#include <QJsonObject>
#include <QObject>
#include <QPointer>

#include <atomic>
#include <functional>

class QProgressDialog;
class QWidget;

namespace strata
{

class FomodDialog;

class InstallDialogs : public QObject, public InstallHost
{
  Q_OBJECT

public:
  explicit InstallDialogs(QWidget* parent);
  ~InstallDialogs() override;

  // Called by Rust, from any thread.
  rust::String ask(rust::Str kind, rust::Str request) override;
  bool progress(std::uint8_t percent) override;

  // On the UI thread: the dialog for one request. Public for the screenshot tests.
  QJsonObject show(const QString& kind, const QJsonObject& request);

private:
  QJsonObject quick(const QJsonObject& r);
  QJsonObject bain(const QJsonObject& r);
  QJsonObject bundle(const QJsonObject& r);
  QJsonObject fomod(const QJsonObject& r);
  QJsonObject manual(const QJsonObject& r);
  QJsonObject rootSort(const QJsonObject& r);
  QJsonObject clash(const QJsonObject& r);
  QJsonObject question(const QString& title, const QString& text);

  QWidget* m_parent;
  QPointer<FomodDialog> m_fomod;
  QPointer<QProgressDialog> m_progress;
  std::atomic<bool> m_cancelled{false};
};

// Installs `archive` on a worker thread with the dialogs above, then calls `done` on the UI
// thread with the JSON report, or with an error message.
void startInstall(QWidget* parent, const QString& root, const QString& profile,
                  const QString& archive, const QString& under,
                  std::function<void(const QString& report, const QString& error)> done);

}  // namespace strata
