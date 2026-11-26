// MO2's OverwriteInfoDialog (overwriteinfodialog.ui): the files in Overwrite, with Open,
// Rename, Delete (to the trash) and New Folder. Every change goes through the core
// (strata_ui::Session's overwrite functions), which refuses while the instance is deployed.
#pragma once

#include <QDialog>
#include <QFileSystemModel>

#include <memory>

namespace Ui
{
class OverwriteInfoDialog;
}

class OverwriteDialog : public QDialog
{
  Q_OBJECT

public:
  OverwriteDialog(const QString& root, const QString& profile, const QString& overwrite,
                  QWidget* parent = nullptr);
  ~OverwriteDialog() override;

  // Whether something in Overwrite changed (the window updates the row and the deploy).
  bool changed() const { return m_changed; }

private:
  QStringList selectedPaths() const;  // relative to Overwrite
  void showMenu(const QPoint& pos);
  void deleteSelected();
  void renameSelected();
  void newFolder();
  bool run(const std::function<void()>& action);

  std::unique_ptr<Ui::OverwriteInfoDialog> ui;
  QFileSystemModel m_model;
  QString m_root;
  QString m_profile;
  QString m_overwrite;
  bool m_changed = false;
};
