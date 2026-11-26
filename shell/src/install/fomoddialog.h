#pragma once

// MO2's FOMOD dialog (installer_fomod's FomodInstallerDialog form) over the Rust session:
// each click is sent to Rust, which answers with the step as it is now (plugin types,
// enabled state, the Next / Install label), so conditions are live.

#include <QDialog>
#include <QEventLoop>
#include <QJsonObject>

namespace Ui
{
class FomodInstallerDialog;
}

namespace strata
{

class FomodDialog : public QDialog
{
  Q_OBJECT

public:
  explicit FomodDialog(QWidget* parent);
  ~FomodDialog() override;

  // Shows `request` (a "fomod" message) and waits for the user's next action.
  QJsonObject wait(const QJsonObject& request);

protected:
  bool eventFilter(QObject* object, QEvent* event) override;
  void closeEvent(QCloseEvent* event) override;

private:
  void build(const QJsonObject& request);
  void answer(QJsonObject a);
  void showPlugin(const QString& description, const QString& image);

  Ui::FomodInstallerDialog* ui;
  QEventLoop m_loop;
  QJsonObject m_answer;
  QString m_images;
  QString m_stepKey;
  bool m_isLast = true;
  bool m_answered = false;  // this request has its answer
};

}  // namespace strata
