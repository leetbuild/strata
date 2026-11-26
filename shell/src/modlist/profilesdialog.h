#pragma once

// MO2's profiles dialog (profilesdialog.ui, ProfilesDialog in profilesdialog.cpp): list,
// create, copy, remove (to the trash), rename, local saves, local inis, select. Every
// action goes through the core; the list is read again after each one.

#include <QDialog>

#include <memory>
#include <optional>

namespace Ui
{
class ProfilesDialog;
}

class QListWidgetItem;

namespace strata::modlist
{

class ProfilesDialog : public QDialog
{
  Q_OBJECT

public:
  ProfilesDialog(const QString& root, QWidget* parent = nullptr);
  ~ProfilesDialog() override;

  // The profile picked with "Select" (now the instance's selected one).
  std::optional<QString> selected() const { return m_selected; }
  // Whether profiles changed (created, renamed, removed, settings).
  bool changed() const { return m_changed; }

private:
  void reload(const QString& current);
  void showCurrent();
  QString currentName() const;
  bool run(const std::function<void()>& action, const QString& failure);
  void addProfile();
  void copyProfile();
  void removeProfile();
  void renameProfile();
  void setLocalSaves(bool on);
  void setLocalInis(bool on);
  void select();

  std::unique_ptr<Ui::ProfilesDialog> ui;
  QString m_root;
  QString m_active;
  std::optional<QString> m_selected;
  bool m_changed = false;
};

}  // namespace strata::modlist
