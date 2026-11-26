#pragma once

// MO2's LootDialog (lootdialog.cpp) with its Loot runner (loot.cpp): the Sort button's modal
// window. While LOOT runs: the step, a busy bar, the log under "Details", Cancel. After: the
// report (general messages, plugin messages, dirty plugins, missing masters and requirements,
// incompatibilities, whether the order changed) and Close.
//
// MO2 spawns lootcli in its VFS and reads its JSON report; Strata calls libloot through the
// core on a worker (crates/strata-ffi/src/bridge_loot.rs) and renders the same markdown in a
// QTextBrowser (MO2 uses a QWebEngineView with marked.js).

#include <QDialog>
#include <QJsonObject>
#include <QStringList>

#include <memory>

namespace Ui
{
class LootDialog;
}

class LootRun;

class LootDialog : public QDialog
{
  Q_OBJECT

public:
  // `updateMasterlist`: download the masterlist before sorting.
  LootDialog(QWidget* parent, const QString& root, const QString& profile, bool updateMasterlist);
  ~LootDialog() override;

  // Starts LOOT, then runs the dialog until it is closed (geometry and the Details expander
  // restored and stored like MO2's).
  int exec() override;
  void accept() override;
  void reject() override;

  // Loot::result: LOOT ran to the end and wrote the order.
  bool succeeded() const { return m_succeeded; }
  bool finished() const { return m_finished; }
  // The report a successful sort returned (lootcli's JSON; empty otherwise).
  const QJsonObject& report() const { return m_report; }
  // The markdown the report view shows.
  const QString& reportMarkdown() const { return m_markdown; }
  void cancel();

protected:
  void closeEvent(QCloseEvent* e) override;

private:
  void createUI();
  void setText(const QString& s);
  void setProgress(int step);
  void log(int level, const QString& line);
  void addLineOutput(const QString& line);
  void onFinished(const QString& json, const QString& error);
  void showReport();
  void openReport();
  void setExpanded(bool open);

  friend class LootRun;

  std::unique_ptr<Ui::LootDialog> ui;
  QString m_root;
  QString m_profile;
  bool m_updateMasterlist;
  std::shared_ptr<LootRun> m_run;
  bool m_started    = false;
  bool m_finished   = false;
  bool m_cancelling = false;
  bool m_succeeded  = false;
  bool m_expanded   = false;
  QStringList m_errors, m_warnings;
  QJsonObject m_report;
  QString m_markdown;
};
