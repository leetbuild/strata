#pragma once

// The delete confirmation of Leet change 8 (MO2 Leet's deletechoice.h): "Move to the
// trash", "Delete permanently" or "Cancel", with "Remember this choice". The remembered
// answer lives in ModOrganizer.ini [DialogChoices] under `action` (MO2's keys: removeMod,
// removeMods), so MO2 and Strata share it.

#include <QDialog>
#include <QString>

class QCheckBox;

namespace strata::modlist
{

enum class DeleteChoice
{
  Cancel,
  Trash,
  Permanent
};

// Asks, unless an answer is remembered for `action` in the instance at `root`.
DeleteChoice askDeleteChoice(QWidget* parent, const QString& root, const QString& title,
                             const QString& text, const QString& action);

// The dialog itself (shown by askDeleteChoice; public for the screenshot hook).
class DeleteConfirmDialog : public QDialog
{
  Q_OBJECT

public:
  DeleteConfirmDialog(QWidget* parent, const QString& title, const QString& text);
  DeleteChoice choice() const { return m_choice; }
  bool remember() const;

private:
  DeleteChoice m_choice = DeleteChoice::Cancel;
  QCheckBox* m_remember;
};

}  // namespace strata::modlist
