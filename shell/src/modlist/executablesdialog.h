#pragma once

// MO2's executables dialog (editexecutablesdialog.ui, EditExecutablesDialog): add (from a
// file, empty, clone), remove, reorder, edit; "create files in mod" per profile. The list
// is edited in memory and stored with OK / Apply through the core (strata_ui::modlist::
// executables::commit, which also rewrites binaries in a mod's Root folder).

#include "strata_bridge/bridge_modlist.h"

#include <QDialog>

#include <memory>
#include <vector>

namespace Ui
{
class EditExecutablesDialog;
}

namespace strata::modlist
{

class ExecutablesDialog : public QDialog
{
  Q_OBJECT

public:
  ExecutablesDialog(const QString& root, const QString& profile, QWidget* parent = nullptr);
  ~ExecutablesDialog() override;

  bool changed() const { return m_changed; }

private:
  struct Row
  {
    QString title, binary, arguments, workingDirectory, steamAppID, outputMod;
    bool toolbar = false, ownIcon = false, hide = false;
  };

  void fillList(int select);
  void showRow(int index);
  void saveRow();  // the edits into the selected row
  int current() const;
  void addNew(Row row);
  void addFromFile();
  void removeCurrent();
  void moveCurrent(int by);
  void browseBinary();
  void browseWorkingDirectory();
  void setTitle(const QString& text);
  bool commit();

  std::unique_ptr<Ui::EditExecutablesDialog> ui;
  QString m_root;
  QString m_profile;
  std::vector<Row> m_rows;
  std::vector<std::string> m_original;  // titles when opened
  bool m_settingUI = false;
  bool m_changed   = false;
};

}  // namespace strata::modlist
