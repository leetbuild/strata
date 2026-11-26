#include "profilesdialog.h"

#include "common.h"
#include "strata_bridge/bridge_modlist.h"
#include "ui_profileinputdialog.h"
#include "ui_profilesdialog.h"

#include <QInputDialog>
#include <QMessageBox>
#include <QPushButton>

namespace strata::modlist
{

namespace
{

// MO2's ProfileInputDialog: a name and "Default Game Settings".
std::optional<std::pair<QString, bool>> askNewProfile(QWidget* parent)
{
  QDialog dialog(parent);
  Ui::ProfileInputDialog ui;
  ui.setupUi(&dialog);
  if (dialog.exec() != QDialog::Accepted || ui.nameEdit->text().trimmed().isEmpty()) {
    return std::nullopt;
  }
  return std::make_pair(ui.nameEdit->text(), ui.defaultSettingsBox->isChecked());
}

}  // namespace

ProfilesDialog::ProfilesDialog(const QString& root, QWidget* parent)
    : QDialog(parent), ui(new Ui::ProfilesDialog), m_root(root)
{
  ui->setupUi(this);
  // Skyrim SE has no BSA invalidation feature (ProfilesDialog's constructor)
  ui->invalidationBox->setEnabled(false);
  ui->invalidationBox->setToolTip(tr("Archive invalidation isn't required for this game."));
  // TransferSavesDialog is not ported yet
  ui->transferButton->setEnabled(false);
  ui->transferButton->setToolTip(tr("Not in Strata yet: move the saves with a file manager."));

  connect(ui->profilesList, &QListWidget::currentItemChanged, this, [this] { showCurrent(); });
  connect(ui->profilesList, &QListWidget::itemActivated, this, [this] { select(); });
  connect(ui->addProfileButton, &QPushButton::clicked, this, [this] { addProfile(); });
  connect(ui->copyProfileButton, &QPushButton::clicked, this, [this] { copyProfile(); });
  connect(ui->removeProfileButton, &QPushButton::clicked, this, [this] { removeProfile(); });
  connect(ui->renameButton, &QPushButton::clicked, this, [this] { renameProfile(); });
  connect(ui->localSavesBox, &QCheckBox::toggled, this, [this](bool on) { setLocalSaves(on); });
  connect(ui->localIniFilesBox, &QCheckBox::toggled, this, [this](bool on) { setLocalInis(on); });
  connect(ui->select, &QPushButton::clicked, this, [this] { select(); });
  connect(ui->close, &QPushButton::clicked, this, &QDialog::close);

  reload({});
}

ProfilesDialog::~ProfilesDialog() = default;

void ProfilesDialog::reload(const QString& current)
{
  QSignalBlocker block(ui->profilesList);
  ui->profilesList->clear();
  try {
    for (const auto& row : profile_rows(utf8(m_root))) {
      auto* item = new QListWidgetItem(qs(row.name), ui->profilesList);
      item->setData(Qt::UserRole, row.local_saves);
      item->setData(Qt::UserRole + 1, row.local_settings);
      item->setData(Qt::UserRole + 2, row.archive_invalidation);
      if (row.selected) {
        m_active = qs(row.name);
      }
    }
  } catch (const std::exception& e) {
    QMessageBox::warning(this, tr("Profiles"), QString::fromUtf8(e.what()));
  }
  const QString want = current.isEmpty() ? m_active : current;
  for (int i = 0; i < ui->profilesList->count(); ++i) {
    if (ui->profilesList->item(i)->text() == want) {
      ui->profilesList->setCurrentRow(i);
    }
  }
  showCurrent();
}

QString ProfilesDialog::currentName() const
{
  auto* item = ui->profilesList->currentItem();
  return item ? item->text() : QString();
}

// on_profilesList_currentItemChanged: the checkboxes follow the selected row, without
// triggering their actions
void ProfilesDialog::showCurrent()
{
  auto* item     = ui->profilesList->currentItem();
  const bool any = item != nullptr;
  QSignalBlocker a(ui->localSavesBox), b(ui->localIniFilesBox), c(ui->invalidationBox);
  ui->localSavesBox->setChecked(any && item->data(Qt::UserRole).toBool());
  ui->localIniFilesBox->setChecked(any && item->data(Qt::UserRole + 1).toBool());
  ui->invalidationBox->setChecked(any && item->data(Qt::UserRole + 2).toBool());
  ui->localSavesBox->setEnabled(any);
  ui->localIniFilesBox->setEnabled(any);
  ui->copyProfileButton->setEnabled(any);
  ui->removeProfileButton->setEnabled(any);
  ui->renameButton->setEnabled(any);
  ui->select->setEnabled(any);
}

bool ProfilesDialog::run(const std::function<void()>& action, const QString& failure)
{
  try {
    action();
    m_changed = true;
    return true;
  } catch (const std::exception& e) {
    QMessageBox::warning(this, tr("Profiles"), failure.arg(QString::fromUtf8(e.what())));
    return false;
  }
}

void ProfilesDialog::addProfile()
{
  const auto input = askNewProfile(this);
  if (!input) {
    return;
  }
  QString made;
  if (run([&] { made = qs(profile_create(utf8(m_root), utf8(input->first), input->second)); },
          tr("failed to create profile: %1"))) {
    reload(made);
  }
}

void ProfilesDialog::copyProfile()
{
  const QString from = currentName();
  bool ok            = false;
  const QString name = QInputDialog::getText(this, tr("Name"), tr("Please enter a name for the new profile"),
                                             QLineEdit::Normal, QString(), &ok);
  if (!ok || name.trimmed().isEmpty()) {
    return;
  }
  QString made;
  if (run([&] { made = qs(profile_copy(utf8(m_root), utf8(from), utf8(name))); },
          tr("failed to copy profile: %1"))) {
    reload(made);
  }
}

void ProfilesDialog::removeProfile()
{
  const QString name = currentName();
  if (name == m_active) {
    QMessageBox::warning(this, tr("Deleting active profile"),
                         tr("Unable to delete active profile.  Please change to a different profile first."));
    return;
  }
  if (QMessageBox::question(this, tr("Confirm"),
                            tr("Are you sure you want to remove this profile (including "
                               "profile-specific save games, if any)? It goes to the trash.")) !=
      QMessageBox::Yes) {
    return;
  }
  if (run([&] { profile_delete(utf8(m_root), utf8(name)); }, tr("failed to remove profile: %1"))) {
    reload({});
  }
}

void ProfilesDialog::renameProfile()
{
  const QString old = currentName();
  if (old == m_active) {
    QMessageBox::warning(this, tr("Renaming active profile"),
                         tr("The active profile cannot be renamed. Please change to a different profile first."));
    return;
  }
  bool ok            = false;
  const QString name = QInputDialog::getText(this, tr("Rename Profile"), tr("New Name"),
                                             QLineEdit::Normal, old, &ok);
  if (!ok || name.trimmed().isEmpty() || name == old) {
    return;
  }
  if (run([&] { profile_rename(utf8(m_root), utf8(old), utf8(name)); }, tr("failed to rename profile: %1"))) {
    reload(name.trimmed());
  }
}

// Profile::enableLocalSaves / enableLocalSettings: switching off asks whether to delete the
// files (Yes: to the trash, No: keep, Cancel: leave it on)
void ProfilesDialog::setLocalSaves(bool on)
{
  const QString name = currentName();
  bool remove        = false;
  if (!on) {
    const auto answer = QMessageBox::question(
        this, tr("Delete profile-specific save games?"),
        tr("Do you want to delete the profile-specific save games? (If you select \"No\", the "
           "save games will show up again if you re-enable profile-specific save games)"),
        QMessageBox::Yes | QMessageBox::No | QMessageBox::Cancel, QMessageBox::No);
    if (answer == QMessageBox::Cancel) {
      QSignalBlocker block(ui->localSavesBox);
      ui->localSavesBox->setChecked(true);
      return;
    }
    remove = answer == QMessageBox::Yes;
  }
  run([&] { profile_set_local_saves(utf8(m_root), utf8(name), on, remove); },
      tr("failed to change local saves: %1"));
  reload(name);
}

void ProfilesDialog::setLocalInis(bool on)
{
  const QString name = currentName();
  bool remove        = false;
  if (!on) {
    const auto answer = QMessageBox::question(
        this, tr("Delete profile-specific game INI files?"),
        tr("Do you want to delete the profile-specific game INI files? (If you select \"No\", the "
           "INI files will be used again if you re-enable profile-specific game INI files.)"),
        QMessageBox::Yes | QMessageBox::No | QMessageBox::Cancel, QMessageBox::No);
    if (answer == QMessageBox::Cancel) {
      QSignalBlocker block(ui->localIniFilesBox);
      ui->localIniFilesBox->setChecked(true);
      return;
    }
    remove = answer == QMessageBox::Yes;
  }
  run([&] { profile_set_local_inis(utf8(m_root), utf8(name), on, remove); },
      tr("failed to change local game INI files: %1"));
  reload(name);
}

// on_select_clicked; Strata selects the profile right away (a live deploy is remounted)
void ProfilesDialog::select()
{
  const QString name = currentName();
  if (name.isEmpty()) {
    return;
  }
  if (name != m_active &&
      !run([&] { profile_select(utf8(m_root), utf8(name)); }, tr("failed to select profile: %1"))) {
    return;
  }
  m_selected = name;
  accept();
}

}  // namespace strata::modlist
