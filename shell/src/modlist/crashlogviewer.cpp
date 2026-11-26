#include "crashlogviewer.h"

#include "common.h"
#include "strata_bridge/bridge_crashlogs.h"

#include <QAction>
#include <QDesktopServices>
#include <QDialogButtonBox>
#include <QFileInfo>
#include <QLabel>
#include <QListWidget>
#include <QMessageBox>
#include <QPushButton>
#include <QUrl>
#include <QVBoxLayout>

namespace strata::modlist
{

CrashLogViewer::CrashLogViewer(const QString& root, const QString& profile, QWidget* parent)
    : QDialog(parent), m_root(root), m_profile(profile)
{
  setObjectName("CrashLogViewer");
  resize(520, 420);
  m_list = new QListWidget(this);
  m_list->setObjectName("crashLogs");
  m_list->setSelectionMode(QAbstractItemView::ExtendedSelection);
  m_list->setDragEnabled(true);

  auto* open = new QAction(tr("&Open"), m_list);
  QFont bold = open->font();
  bold.setBold(true);
  open->setFont(bold);
  connect(open, &QAction::triggered, this, [this] { openSelected(); });
  m_list->addAction(open);
  auto* remove = new QAction(tr("&Delete"), m_list);
  connect(remove, &QAction::triggered, this, [this] { deleteSelected(); });
  m_list->addAction(remove);
  m_list->setContextMenuPolicy(Qt::ActionsContextMenu);
  connect(m_list, &QListWidget::itemActivated, this, [this] { openSelected(); });

  m_status = new QLabel(this);
  m_status->setObjectName("crashLogStatus");
  m_status->setWordWrap(true);

  auto* buttons = new QDialogButtonBox(QDialogButtonBox::Close, Qt::Horizontal, this);
  buttons->button(QDialogButtonBox::Close)->setAutoDefault(false);
  auto* update = buttons->addButton(tr("Update names database"), QDialogButtonBox::ActionRole);
  update->setAutoDefault(false);
  connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
  connect(update, &QPushButton::clicked, this, [this] {
    try {
      const QString count = qs(strata::crashlogs::crash_logs_update_database());
      const QString labelled = qs(strata::crashlogs::crash_logs_label(rs(m_root), rs(m_profile)));
      m_status->setText(tr("names database: %1. %2").arg(count, labelled));
    } catch (const std::exception& e) {
      QMessageBox::warning(this, tr("Update names database"), QString::fromUtf8(e.what()));
    }
  });

  auto* layout = new QVBoxLayout(this);
  layout->addWidget(m_list);
  layout->addWidget(m_status);
  layout->addWidget(buttons);

  try {
    m_status->setText(qs(strata::crashlogs::crash_logs_label(rs(m_root), rs(m_profile))));
  } catch (const std::exception& e) {
    m_status->setText(QString::fromUtf8(e.what()));
  }
  reload();
}

void CrashLogViewer::reload()
{
  m_list->clear();
  try {
    const auto view = strata::crashlogs::crash_log_folder(rs(m_root), rs(m_profile));
    const QString folder = qs(view.folder);
    setWindowTitle(tr("Crash Log Viewer - %1").arg(folder));
    for (const auto& log : view.logs) {
      const QString path = qs(log);
      auto* item = new QListWidgetItem(QFileInfo(path).fileName(), m_list);
      item->setData(Qt::UserRole, path);
      item->setToolTip(path);
    }
    if (folder.isEmpty()) {
      m_status->setText(tr("No crash log folder: crash logs are read for Skyrim Special Edition with a prefix."));
    }
  } catch (const std::exception& e) {
    setWindowTitle(tr("Crash Log Viewer"));
    m_status->setText(QString::fromUtf8(e.what()));
  }
}

void CrashLogViewer::openSelected()
{
  for (auto* item : m_list->selectedItems()) {
    QDesktopServices::openUrl(QUrl::fromLocalFile(item->data(Qt::UserRole).toString()));
  }
}

void CrashLogViewer::deleteSelected()
{
  for (auto* item : m_list->selectedItems()) {
    try {
      strata::crashlogs::crash_log_trash(rs(item->data(Qt::UserRole).toString()));
    } catch (const std::exception& e) {
      QMessageBox::warning(this, tr("Delete"), QString::fromUtf8(e.what()));
      break;
    }
  }
  reload();
}

}  // namespace strata::modlist
