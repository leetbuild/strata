#include "overwritedialog.h"

#include "strata_bridge/bridge.h"
#include "ui_overwriteinfodialog.h"

#include <QDesktopServices>
#include <QInputDialog>
#include <QMenu>
#include <QMessageBox>
#include <QShortcut>
#include <QUrl>

namespace
{

QString qs(const rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

rust::Str rstr(const QByteArray& utf8)
{
  return rust::Str(utf8.data(), std::size_t(utf8.size()));
}

}  // namespace

OverwriteDialog::OverwriteDialog(const QString& root, const QString& profile, const QString& overwrite,
                                 QWidget* parent)
    : QDialog(parent), ui(std::make_unique<Ui::OverwriteInfoDialog>()), m_root(root),
      m_profile(profile), m_overwrite(overwrite)
{
  ui->setupUi(this);
  // MO2's hint is about dragging files onto mods, which Strata doesn't do yet
  ui->label->setText(tr("To move these files into a mod, use \"Create Mod...\" or \"Move content to Mod...\" "
                        "in Overwrite's menu in the mod list."));
  // QFileSystemModel only shows; the changes go through the core
  m_model.setReadOnly(true);
  m_model.setFilter(QDir::AllEntries | QDir::NoDotAndDotDot | QDir::Hidden);
  m_model.setRootPath(overwrite);
  ui->filesView->setModel(&m_model);
  ui->filesView->setRootIndex(m_model.index(overwrite));
  ui->filesView->setColumnWidth(0, 250);
  ui->filesView->setSelectionMode(QAbstractItemView::ExtendedSelection);
  ui->filesView->setContextMenuPolicy(Qt::CustomContextMenu);
  ui->filesView->sortByColumn(0, Qt::AscendingOrder);
  connect(ui->filesView, &QWidget::customContextMenuRequested, this, &OverwriteDialog::showMenu);
  connect(ui->explorerButton, &QPushButton::clicked, this,
          [this] { QDesktopServices::openUrl(QUrl::fromLocalFile(m_overwrite)); });
  // the form's button box closes it
  auto* del = new QShortcut(QKeySequence::Delete, this);
  connect(del, &QShortcut::activated, this, [this] {
    if (ui->filesView->hasFocus()) {
      deleteSelected();
    }
  });
}

OverwriteDialog::~OverwriteDialog() = default;

QStringList OverwriteDialog::selectedPaths() const
{
  QStringList out;
  const QDir base(m_overwrite);
  for (const QModelIndex& index : ui->filesView->selectionModel()->selectedRows(0)) {
    out << base.relativeFilePath(m_model.filePath(index));
  }
  return out;
}

bool OverwriteDialog::run(const std::function<void()>& action)
{
  try {
    action();
    m_changed = true;
    return true;
  } catch (const std::exception& e) {
    QMessageBox::warning(this, windowTitle(), QString::fromUtf8(e.what()));
    return false;
  }
}

// on_filesView_customContextMenuRequested: New Folder always; Open for files; Rename and
// Delete for a selection
void OverwriteDialog::showMenu(const QPoint& pos)
{
  const QModelIndex at = ui->filesView->indexAt(pos);
  QMenu menu(this);
  menu.addAction(tr("&New Folder"), this, [this] { newFolder(); });
  const QStringList selected = selectedPaths();
  if (at.isValid() && !selected.isEmpty()) {
    if (!m_model.isDir(at)) {
      menu.addAction(tr("&Open"), this, [this] {
        for (const QModelIndex& index : ui->filesView->selectionModel()->selectedRows(0)) {
          QDesktopServices::openUrl(QUrl::fromLocalFile(m_model.filePath(index)));
        }
      });
    }
    menu.addAction(tr("&Rename"), this, [this] { renameSelected(); });
    menu.addAction(tr("&Delete"), this, [this] { deleteSelected(); });
  }
  menu.exec(ui->filesView->viewport()->mapToGlobal(pos));
}

void OverwriteDialog::deleteSelected()
{
  const QStringList paths = selectedPaths();
  if (paths.isEmpty()) {
    return;
  }
  const QString question = paths.size() == 1
                               ? tr("Are you sure you want to delete \"%1\"?").arg(QFileInfo(paths[0]).fileName())
                               : tr("Are you sure you want to delete the selected files?");
  if (QMessageBox::question(this, tr("Confirm"), question + "\n" + tr("They go to the trash."),
                            QMessageBox::Yes | QMessageBox::No) != QMessageBox::Yes) {
    return;
  }
  rust::Vec<rust::String> list;
  for (const QString& p : paths) {
    list.push_back(rust::String(p.toStdString()));
  }
  run([&] { strata::open_session(m_root.toStdString(), m_profile.toStdString())->remove_from_overwrite(std::move(list), true); });
}

void OverwriteDialog::renameSelected()
{
  const QStringList paths = selectedPaths();
  if (paths.size() != 1) {
    return;
  }
  bool ok            = false;
  const QString old  = QFileInfo(paths[0]).fileName();
  const QString name = QInputDialog::getText(this, tr("Rename"), tr("New name:"), QLineEdit::Normal, old, &ok);
  if (!ok || name.isEmpty() || name == old) {
    return;
  }
  const QByteArray from = paths[0].toUtf8();
  const QByteArray to   = name.toUtf8();
  run([&] { strata::open_session(m_root.toStdString(), m_profile.toStdString())->rename_in_overwrite(rstr(from), rstr(to)); });
}

// createDirectoryTriggered: in the selected folder (or the selected file's), else at the top
void OverwriteDialog::newFolder()
{
  QString dir;
  const QModelIndex current = ui->filesView->currentIndex();
  if (current.isValid() && ui->filesView->selectionModel()->isSelected(current)) {
    const QString path = m_model.isDir(current) ? m_model.filePath(current) : QFileInfo(m_model.filePath(current)).path();
    dir = QDir(m_overwrite).relativeFilePath(path);
    if (dir == ".") {
      dir.clear();
    }
  }
  const QByteArray d = dir.toUtf8();
  QString made;
  if (run([&] { made = qs(strata::open_session(m_root.toStdString(), m_profile.toStdString())->new_folder_in_overwrite(rstr(d))); })) {
    const QModelIndex index = m_model.index(m_overwrite + "/" + made);
    ui->filesView->setCurrentIndex(index);
  }
}
