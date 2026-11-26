#include "deleteconfirm.h"

#include "common.h"
#include "strata_bridge/bridge_modlist.h"

#include <QApplication>
#include <QCheckBox>
#include <QDialogButtonBox>
#include <QHBoxLayout>
#include <QLabel>
#include <QPushButton>
#include <QStyle>
#include <QVBoxLayout>

namespace strata::modlist
{

DeleteConfirmDialog::DeleteConfirmDialog(QWidget* parent, const QString& title,
                                         const QString& text)
    : QDialog(parent)
{
  setWindowTitle(title);
  auto* layout = new QVBoxLayout(this);
  auto* top    = new QHBoxLayout;
  auto* icon   = new QLabel(this);
  const int size = style()->pixelMetric(QStyle::PM_MessageBoxIconSize, nullptr, this);
  icon->setPixmap(style()->standardIcon(QStyle::SP_MessageBoxQuestion).pixmap(size, size));
  icon->setAlignment(Qt::AlignTop);
  top->addWidget(icon);
  auto* label = new QLabel(text, this);
  label->setTextFormat(Qt::RichText);
  label->setWordWrap(true);
  top->addWidget(label, 1);
  layout->addLayout(top);

  m_remember = new QCheckBox(tr("Remember this choice"), this);
  layout->addWidget(m_remember);

  // TaskDialog's buttons, in MO2 Leet's order
  auto* buttons   = new QDialogButtonBox(this);
  auto* trash     = buttons->addButton(tr("Move to the trash"), QDialogButtonBox::AcceptRole);
  auto* permanent = buttons->addButton(tr("Delete permanently"), QDialogButtonBox::DestructiveRole);
  buttons->addButton(QDialogButtonBox::Cancel);
  trash->setDefault(true);
  connect(trash, &QPushButton::clicked, this, [this] {
    m_choice = DeleteChoice::Trash;
    accept();
  });
  connect(permanent, &QPushButton::clicked, this, [this] {
    m_choice = DeleteChoice::Permanent;
    accept();
  });
  connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
  layout->addWidget(buttons);
}

bool DeleteConfirmDialog::remember() const
{
  return m_remember->isChecked();
}

DeleteChoice askDeleteChoice(QWidget* parent, const QString& root, const QString& title,
                             const QString& text, const QString& action)
{
  switch (delete_choice(utf8(root), utf8(action))) {
  case 1:
    return DeleteChoice::Trash;
  case 2:
    return DeleteChoice::Permanent;
  default:
    break;
  }

  DeleteConfirmDialog dialog(parent, title, text);
  dialog.exec();
  const auto choice = dialog.choice();
  // a remembered Cancel would make the action do nothing without asking: not stored
  if (dialog.remember() && choice != DeleteChoice::Cancel) {
    try {
      set_delete_choice(utf8(root), utf8(action), choice == DeleteChoice::Trash ? 1 : 2);
    } catch (const std::exception& e) {
      qWarning("cannot remember the choice: %s", e.what());
    }
  }
  return choice;
}

}  // namespace strata::modlist
