#include "pluginstab.h"

#include "loot/lootdialog.h"
#include "loot/lootreport.h"
#include "modlist.h"
#include "ui_mainwindow.h"

#include <QApplication>
#include <QBuffer>
#include <QDateTime>
#include <QHeaderView>
#include <QImage>
#include <QInputDialog>
#include <QJsonArray>
#include <QLocale>
#include <QMenu>
#include <QMessageBox>
#include <QPointer>
#include <QTreeWidgetItem>

#include <thread>

namespace
{

constexpr int SortRole = Qt::UserRole + 10;

enum Column
{
  ColName,
  ColFlags,
  ColPriority,
  ColIndex,
};

QString qs(const rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

rust::Vec<rust::String> rustNames(const QStringList& names)
{
  rust::Vec<rust::String> out;
  for (const QString& n : names) {
    out.push_back(rust::String(n.toStdString()));
  }
  return out;
}

QStandardItem* cell(const QString& text, const QVariant& sortKey = {})
{
  auto* item = new QStandardItem(text);
  item->setEditable(false);
  item->setDropEnabled(false);
  item->setData(sortKey.isValid() ? sortKey : QVariant(text.toLower()), SortRole);
  return item;
}

}  // namespace

PluginsTab::PluginsTab(Ui::MainWindow* ui, QObject* parent) : QObject(parent), ui(ui)
{
  m_model.setHorizontalHeaderLabels({tr("Name"), tr("Flags"), tr("Priority"), tr("Mod Index")});
  m_proxy.setSourceModel(&m_model);
  m_proxy.setSortRole(SortRole);
  m_proxy.setFilterCaseSensitivity(Qt::CaseInsensitive);
  m_proxy.setFilterKeyColumn(ColName);
  ui->espList->setModel(&m_proxy);
  ui->espList->setSortingEnabled(true);
  ui->espList->sortByColumn(ColPriority, Qt::AscendingOrder);
  ui->espList->setRootIsDecorated(false);
  ui->espList->setSelectionMode(QAbstractItemView::ExtendedSelection);
  ui->espList->header()->setStretchLastSection(true);
  ui->espList->header()->resizeSection(ColName, 300);
  ui->espList->setItemDelegateForColumn(ColFlags, new IconDelegate(ui->espList, ColFlags, 60));
  ui->espList->setContextMenuPolicy(Qt::CustomContextMenu);
  connect(ui->espList, &QWidget::customContextMenuRequested, this, &PluginsTab::showContextMenu);
  connect(ui->espList, &PluginListView::dropRequested, this, &PluginsTab::onDrop);
  connect(ui->espFilterEdit, &QLineEdit::textChanged, &m_proxy,
          &QSortFilterProxyModel::setFilterFixedString);
  connect(&m_model, &QStandardItemModel::itemChanged, this, &PluginsTab::onItemChanged);
  ui->sortButton->setEnabled(true);
  connect(ui->sortButton, &QPushButton::clicked, this, &PluginsTab::sort);
  ui->savegameList->setRootIsDecorated(false);
}

void PluginsTab::setSource(const QString& root, const QString& profile)
{
  m_root    = root;
  m_profile = profile;
  m_plugins.reset();
  // another profile: no row of the old one stays clickable while the new one loads
  m_updating = true;
  m_model.removeRows(0, m_model.rowCount());
  m_updating = false;
  reload();
}

void PluginsTab::reload()
{
  if (m_root.isEmpty()) {
    return;
  }
  const int generation      = ++m_generation;
  m_loading                 = true;
  const std::string root    = m_root.toStdString();
  const std::string profile = m_profile.toStdString();
  QPointer<PluginsTab> self(this);
  std::thread([self, generation, root, profile] {
    std::shared_ptr<rust::Box<strata::Plugins>> plugins;
    auto saves = std::make_shared<rust::Vec<strata::SaveRow>>();
    QString error;
    try {
      auto session = strata::open_session(root, profile);
      plugins      = std::make_shared<rust::Box<strata::Plugins>>(session->build_plugins());
      *saves       = session->saves(**plugins);
    } catch (const std::exception& e) {
      error = QString::fromUtf8(e.what());
    }
    QMetaObject::invokeMethod(
        qApp,
        [self, generation, plugins, saves, error] {
          if (!self || generation != self->m_generation) {
            return;
          }
          self->m_loading = false;
          if (self->m_again) {
            // a change was made while this was read: what was read may miss it
            self->m_again = false;
            self->reload();
            return;
          }
          if (!error.isEmpty()) {
            emit self->error(tr("plugins: %1").arg(error));
          }
          if (plugins) {
            self->m_plugins = plugins;
            self->fill();
          }
          self->fillSaves(*saves);
        },
        Qt::QueuedConnection);
  }).detach();
}

void PluginsTab::fill()
{
  if (!m_plugins) {
    return;
  }
  const QStringList keep = selected();
  m_updating = true;
  m_proxy.setDynamicSortFilter(false);
  m_model.removeRows(0, m_model.rowCount());
  int active = 0;
  for (const auto& p : (*m_plugins)->rows()) {
    auto* name = cell(qs(p.name));
    name->setCheckable(!p.force_loaded && !p.force_disabled);
    name->setData(p.enabled ? Qt::Checked : Qt::Unchecked, Qt::CheckStateRole);
    name->setDragEnabled(!p.force_loaded);
    // PluginList::m_AdditionalInfo: the last LOOT report's part for this plugin
    const auto lootInfo = m_loot.constFind(qs(p.name).toLower());
    const QString tooltip =
        qs(p.tooltip) + (lootInfo != m_loot.cend() ? loot::pluginTooltip(*lootInfo) : QString());
    name->setToolTip(tooltip);
    QFont font;
    font.setBold(p.bold);
    font.setItalic(p.italic);
    name->setFont(font);
    // PluginList::foregroundData
    if (p.force_loaded) {
      name->setForeground(QBrush(Qt::gray));
    } else if (p.force_disabled) {
      name->setForeground(QBrush(Qt::darkRed));
    }
    auto* flags = cell(QString());
    QStringList icons;
    for (const auto& i : p.icons) {
      icons << qs(i);
    }
    if (lootInfo != m_loot.cend()) {
      icons = loot::pluginIcons(icons, *lootInfo);
    }
    flags->setData(icons, modlist::IconsRole);
    flags->setToolTip(tooltip);
    auto* priority = cell(QString::number(p.priority), qint64(p.priority));
    priority->setTextAlignment(Qt::AlignCenter);
    auto* index = cell(qs(p.index), qint64(p.priority));
    index->setTextAlignment(Qt::AlignCenter);
    m_model.appendRow({name, flags, priority, index});
    active += p.enabled ? 1 : 0;
  }
  m_updating = false;
  m_proxy.setDynamicSortFilter(true);
  m_proxy.invalidate();
  ui->activePluginsCounter->display(active);
  fillArchives();
  QItemSelection selection;
  for (int r = 0; r < m_proxy.rowCount(); ++r) {
    const QModelIndex index = m_proxy.index(r, ColName);
    if (keep.contains(index.data().toString())) {
      selection.select(index, index.siblingAtColumn(ColIndex));
    }
  }
  ui->espList->selectionModel()->select(selection, QItemSelectionModel::ClearAndSelect);
  emit filled();
}

void PluginsTab::fillArchives()
{
  // MainWindow::updateBSAList: one item per mod, its archives below, all read only
  ui->bsaList->clear();
  ui->bsaList->setHeaderLabels({tr("Archive"), tr("Mod")});
  QHash<QString, QTreeWidgetItem*> groups;
  for (const auto& a : (*m_plugins)->archives()) {
    const QString origin = qs(a.origin);
    QTreeWidgetItem*& group = groups[origin];
    if (!group) {
      group = new QTreeWidgetItem(ui->bsaList, {origin});
      group->setFlags(group->flags() & ~Qt::ItemIsDragEnabled);
    }
    auto* item = new QTreeWidgetItem(group, {qs(a.name), origin});
    item->setFlags(item->flags() & ~(Qt::ItemIsDropEnabled | Qt::ItemIsUserCheckable));
    item->setCheckState(0, a.loaded ? Qt::Checked : Qt::Unchecked);
    item->setDisabled(true);
  }
  ui->bsaList->expandAll();
}

void PluginsTab::fillSaves(const rust::Vec<strata::SaveRow>& saves)
{
  ui->savegameList->clear();
  for (const auto& s : saves) {
    auto* item = new QTreeWidgetItem(ui->savegameList, {qs(s.name), qs(s.file)});
    QString tip = qs(s.tooltip);
    if (s.width > 0 && s.height > 0 && s.screenshot.size() >= std::size_t(s.width) * s.height * 4) {
      // the save's screenshot above the text, as MO2's save info widget shows it
      const QImage image(s.screenshot.data(), int(s.width), int(s.height), int(s.width) * 4,
                         QImage::Format_RGBA8888_Premultiplied);
      QByteArray png;
      QBuffer buffer(&png);
      buffer.open(QIODevice::WriteOnly);
      image.scaledToWidth(std::min(320, int(s.width)), Qt::SmoothTransformation).save(&buffer, "PNG");
      tip = QStringLiteral("<img src=\"data:image/png;base64,%1\"><br>").arg(QString::fromLatin1(png.toBase64())) + tip;
    }
    if (s.created >= 0) {
      tip += "<br>" + QLocale().toString(QDateTime::fromSecsSinceEpoch(s.created), QLocale::ShortFormat);
    }
    item->setToolTip(0, tip);
    if (!s.missing.empty()) {
      item->setForeground(0, QBrush(Qt::red));
    }
  }
}

QStringList PluginsTab::selected() const
{
  QStringList names;
  if (ui->espList->selectionModel()) {
    for (const QModelIndex& index : ui->espList->selectionModel()->selectedRows(ColName)) {
      names << index.data().toString();
    }
  }
  return names;
}

template <class Op>
void PluginsTab::operation(const char* what, Op op)
{
  if (!m_plugins) {
    return;
  }
  m_again = m_again || m_loading;
  try {
    op(**m_plugins);
  } catch (const std::exception& e) {
    emit error(QStringLiteral("%1: %2").arg(QString::fromUtf8(what), QString::fromUtf8(e.what())));
  }
  fill();  // the core's order and states (masters first, locked positions...)
}

void PluginsTab::onItemChanged(QStandardItem* item)
{
  if (m_updating || item->column() != ColName || !item->isCheckable()) {
    return;
  }
  const QString name = item->text();
  const bool enabled = item->checkState() == Qt::Checked;
  operation("enable plugin", [&](strata::Plugins& p) { p.set_enabled(rustNames({name}), enabled); });
}

void PluginsTab::onDrop(int row)
{
  const QStringList names = selected();
  if (names.isEmpty()) {
    return;
  }
  const auto* header = ui->espList->header();
  if (header->sortIndicatorSection() != ColPriority) {
    emit error(tr("Drag&Drop is only supported when sorting by priority"));
    return;
  }
  const bool ascending = header->sortIndicatorOrder() == Qt::AscendingOrder;
  qint64 priority      = ascending ? -1 : 0;
  if (row >= 0) {
    const qint64 p = m_proxy.index(row, ColPriority).data(SortRole).toLongLong();
    priority       = ascending ? p : p + 1;
  }
  operation("move plugins", [&](strata::Plugins& p) { p.send_to_priority(rustNames(names), priority); });
}

void PluginsTab::showContextMenu(const QPoint& pos)
{
  const QStringList names = selected();
  QMenu menu;
  // PluginListContextMenu, the actions the core has
  if (!names.isEmpty()) {
    menu.addAction(tr("Enable selected"), this, [this, names] {
      operation("enable", [&](strata::Plugins& p) { p.set_enabled(rustNames(names), true); });
    });
    menu.addAction(tr("Disable selected"), this, [this, names] {
      operation("disable", [&](strata::Plugins& p) { p.set_enabled(rustNames(names), false); });
    });
    menu.addSeparator();
  }
  // asked first, as MO2 does; the primary plugins stay enabled (PluginList::setEnabledAll)
  menu.addAction(tr("Enable all"), this, [this] {
    if (QMessageBox::question(ui->espList->window(), tr("Confirm"), tr("Really enable all plugins?"),
                              QMessageBox::Yes | QMessageBox::No) == QMessageBox::Yes) {
      operation("enable all", [&](strata::Plugins& p) { p.set_enabled_all(true); });
    }
  });
  menu.addAction(tr("Disable all"), this, [this] {
    if (QMessageBox::question(ui->espList->window(), tr("Confirm"), tr("Really disable all plugins?"),
                              QMessageBox::Yes | QMessageBox::No) == QMessageBox::Yes) {
      operation("disable all", [&](strata::Plugins& p) { p.set_enabled_all(false); });
    }
  });
  if (names.isEmpty()) {
    menu.exec(ui->espList->viewport()->mapToGlobal(pos));
    return;
  }
  menu.addSeparator();
  QMenu* send = menu.addMenu(tr("Send to... "));
  send->addAction(tr("Top"), this, [this, names] {
    operation("send to top", [&](strata::Plugins& p) { p.send_to_priority(rustNames(names), 0); });
  });
  send->addAction(tr("Bottom"), this, [this, names] {
    operation("send to bottom", [&](strata::Plugins& p) { p.send_to_priority(rustNames(names), -1); });
  });
  send->addAction(tr("Priority..."), this, [this, names] {
    bool ok      = false;
    const int to = QInputDialog::getInt(nullptr, tr("Set Priority"), tr("Set the priority of the selected plugins"),
                                        0, 0, m_model.rowCount(), 1, &ok);
    if (ok) {
      operation("send to priority", [&](strata::Plugins& p) { p.send_to_priority(rustNames(names), to); });
    }
  });
  menu.addSeparator();
  menu.addAction(tr("Lock load order"), this, [this, names] {
    operation("lock", [&](strata::Plugins& p) {
      for (const QString& n : names) {
        const QByteArray u = n.toUtf8();
        p.set_locked(rust::Str(u.data(), std::size_t(u.size())), true);
      }
    });
  });
  menu.addAction(tr("Unlock load order"), this, [this, names] {
    operation("unlock", [&](strata::Plugins& p) {
      for (const QString& n : names) {
        const QByteArray u = n.toUtf8();
        p.set_locked(rust::Str(u.data(), std::size_t(u.size())), false);
      }
    });
  });
  menu.exec(ui->espList->viewport()->mapToGlobal(pos));
}

// PluginListView::onSortButtonClicked and runLoot: asks, then LOOT runs in its modal dialog;
// the report's plugin parts go to the list's flags and tooltips until the next sort.
void PluginsTab::sort()
{
  QWidget* top = ui->espList->window();
  bool offline = false;
  try {
    const auto v = strata::open_session(m_root.toStdString(), m_profile.toStdString())
                       ->load_setting("Settings", "offline_mode");
    const QByteArray t =
        QByteArray(reinterpret_cast<const char*>(v.bytes.data()), qsizetype(v.bytes.size()))
            .trimmed()
            .toLower();
    offline = v.found && !(t.isEmpty() || t == "0" || t == "false");
  } catch (const std::exception&) {
  }
  QString question = tr("Are you sure you want to sort your plugins list?");
  if (offline) {
    question += "\r\n\r\n" + tr("Note: You are currently in offline mode and LOOT will not update "
                                  "the master list.");
  }
  if (QMessageBox::question(top, tr("Sorting plugins"), question,
                            QMessageBox::Yes | QMessageBox::No) != QMessageBox::Yes) {
    return;
  }

  // the list is saved on every change: nothing to save first (MO2: savePluginList)
  LootDialog dialog(top, m_root, m_profile, !offline && !m_didUpdateMasterList);
  dialog.exec();
  if (dialog.succeeded()) {
    // don't assume the master list was updated in offline mode
    if (!offline) {
      m_didUpdateMasterList = true;
    }
    // LootDialog::showReport: clearAdditionalInformation, addLootReport
    m_loot.clear();
    for (const auto& v : dialog.report().value("plugins").toArray()) {
      const QJsonObject plugin = v.toObject();
      m_loot.insert(plugin.value("name").toString().toLower(), plugin);
    }
  }
  reload();
}

void PluginsTab::toggleForTest(const QString& name)
{
  const auto found = m_model.findItems(name);
  if (!found.isEmpty()) {
    auto* item = found.first();
    item->setCheckState(item->checkState() == Qt::Checked ? Qt::Unchecked : Qt::Checked);
  }
}
