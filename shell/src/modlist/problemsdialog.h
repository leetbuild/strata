#pragma once

// The Problems dialog (MO2's ProblemsDialog, problemsdialog.ui, titled "Notifications"): the
// problems of strata-diag's checks, a description pane, and a Fix button where the core
// has a fix (strata::builtins::problem_fix). The checks build the merged tree, so they run
// on a worker thread; the list says "Checking..." meanwhile.

#include <QDialog>

#include <memory>

namespace Ui
{
class ProblemsDialog;
}

namespace strata::modlist
{

class ProblemsDialog : public QDialog
{
  Q_OBJECT

public:
  ProblemsDialog(const QString& root, const QString& profile, QWidget* parent = nullptr);
  ~ProblemsDialog() override;

  // Whether a fix changed something the window shows (the plugin list).
  bool changed() const { return m_changed; }

signals:
  // The checks are done (also after a fix); `count` problems were found.
  void diagnosed(int count);

private:
  struct Row
  {
    QString id, summary, description, fix;
  };

  void runDiagnosis();
  void showRows(const std::vector<Row>& rows, const QString& error);
  void fix(const QString& id);

  std::unique_ptr<Ui::ProblemsDialog> ui;
  QString m_root;
  QString m_profile;
  bool m_changed = false;
};

}  // namespace strata::modlist
