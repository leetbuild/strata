#pragma once

// MO2's settings dialog (settingsdialog.ui, SettingsDialog and its tabs). Which widget
// edits which setting is strata_ui::modlist::settings::WIDGETS; the values come from and go
// back to ModOrganizer.ini through the core, one line each. Widgets without a meaning on
// Linux are disabled with a tooltip saying why. A "Leet" tab holds Leet's settings.

#include "install/installhost_api.h"

#include <QDialog>
#include <QMap>
#include <QPointer>

#include <functional>
#include <memory>

class QThread;

namespace Ui
{
class SettingsDialog;
}

namespace strata::modlist
{

class SettingsDialog : public QDialog, public NexusLoginHost
{
  Q_OBJECT

public:
  SettingsDialog(const QString& root, QWidget* parent = nullptr);
  ~SettingsDialog() override;

  bool changed() const { return m_changed; }

  // NexusLoginHost, on the login's worker thread
  void state(rust::Str line, rust::Str url) override;

private:
  // the Nexus tab (MO2's NexusConnectionUI and NexusSettingsTab::updateNexusData)
  enum class NexusBusy { None, Login, Check };
  void setupNexus();
  void nexusConnect();
  void nexusManual();
  void nexusDisconnect();
  void nexusUpdateState();
  void nexusShowAccount(const QString& json);
  void nexusLog(const QString& lines);
  // runs `work` on a worker thread; `done(result, error)` on the UI thread afterwards
  void nexusRun(NexusBusy busy, std::function<QString()> work, std::function<void(QString, QString)> done);

  void addLeetTab();
  void fillChoices();
  void disableUnported();
  void fillColorTable();
  void load();
  bool store();
  void editList(const QString& widget, const QString& title, const QString& separator);

  std::unique_ptr<Ui::SettingsDialog> ui;
  QString m_root;
  QMap<QString, QString> m_lists;  // the list buttons' values (one item per line)
  bool m_changed = false;
  NexusBusy m_nexusBusy = NexusBusy::None;
  bool m_hasKey = false;
  QPointer<QThread> m_nexusWorker;
};

}  // namespace strata::modlist
